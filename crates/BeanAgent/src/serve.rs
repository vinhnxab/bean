//! `BeanAgent serve` — chạy web và Telegram qua cùng một Router.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::serve;
use beanagent_channels::TelegramChannel;
use beanagent_core::{Channel, Router, RouterDeps, Scheduler};
use beanagent_memory::{SqliteStore, Store};
use beanagent_skills::SkillCatalog;
use beanagent_types::Config;
use beanagent_web::{AuthService, WebChannel, WebState, build_router};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::chat;
use crate::cli::{ChatArgs, ServeArgs};

/// Chạy các kênh được bật trong cấu hình trên một Router/store chung.
pub async fn run(args: &ServeArgs, config_path: Option<&Path>) -> Result<()> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let web_enabled = config.web.enabled;
    let telegram_enabled = config.telegram.enabled;
    if !web_enabled && !telegram_enabled {
        bail!("không có kênh nào được bật: bật web.enabled hoặc telegram.enabled");
    }
    if web_enabled && !config.web.bind.ip().is_loopback() {
        if !config.web.allow_remote {
            bail!(
                "web.bind ngoài loopback yêu cầu [web].allow_remote = true; nếu public, phải đặt sau reverse proxy TLS/Tailscale/VPN"
            );
        }
        tracing::warn!(
            bind = %config.web.bind,
            origin = %config.web.public_origin,
            "web đang bind ngoài loopback; phải đặt sau reverse proxy TLS/Tailscale/VPN"
        );
    }
    tracing::info!(
        provider = config.llm.provider.as_str(),
        model = %config.llm.model,
        web_enabled,
        telegram_enabled,
        "khởi động BeanAgent"
    );

    std::fs::create_dir_all(&config.agent.workspace).with_context(|| {
        format!(
            "không tạo được workspace {}",
            config.agent.workspace.display()
        )
    })?;
    std::fs::create_dir_all(&config.data.dir)
        .with_context(|| format!("không tạo được data.dir {}", config.data.dir.display()))?;

    let store = Arc::new(
        SqliteStore::open(&chat::store_path(&config))
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .context("không mở được SQLite store")?,
    );
    let store_dyn: Arc<dyn Store> = store.clone();
    let auth = if web_enabled {
        Some(AuthService::load(&config, store_dyn.clone()).context(
            "web chưa được bật: chạy `BeanAgent auth set-password` trước (auth.toml phải là argon2id, 0600)",
        )?)
    } else {
        None
    };

    let chat_args = ChatArgs {
        fake_llm: args.fake_llm.clone(),
        workspace: None,
    };
    let (provider, web_search_api_key) = chat::build_provider(&chat_args, &config)?;
    let telegram_token = config
        .resolve_telegram_token()
        .context("đọc TELEGRAM_BOT_TOKEN thất bại")?;
    let user_skills_root = chat::expand_tilde(&config.data.dir).join("skills");
    let skills = SkillCatalog::load_with_create_root(
        &[std::path::PathBuf::from("skills"), user_skills_root.clone()],
        user_skills_root,
    );
    let skills_index = if config.tools.enabled.iter().any(|group| group == "skills") {
        skills.index()
    } else {
        String::new()
    };
    let built =
        chat::build_registry(&config, store.clone(), skills.clone(), web_search_api_key).await?;
    let mcp = built.mcp;
    let workspace = built.registry.workspace_opt();
    let registry = Arc::new(built.registry);
    let audit = chat::build_audit(&config);
    let router = Arc::new(Router::new(RouterDeps {
        config: config.clone(),
        store: store_dyn.clone(),
        registry,
        llm: provider,
        audit: audit.clone(),
        skills_index,
    }));
    let scheduler = Arc::new(
        Scheduler::new(
            store_dyn.clone(),
            router.clone(),
            config.agent.timezone.clone(),
        )
        .context("không khởi tạo được scheduler")?,
    );

    let mut web_listener = None;
    if web_enabled {
        let state = WebState::new(
            config.clone(),
            store_dyn.clone(),
            router.clone(),
            auth.context("web thiếu auth service")?,
            audit.clone(),
            skills.clone(),
            workspace.clone(),
        )?;
        let channel = Arc::new(WebChannel::new(state.notifications.clone()));
        router
            .register_channel(channel)
            .context("đăng ký WebChannel thất bại")?;
        let listener = TcpListener::bind(config.web.bind)
            .await
            .with_context(|| format!("không bind được {}", config.web.bind))?;
        let local_addr = listener
            .local_addr()
            .context("đọc địa chỉ listener thất bại")?;
        tracing::info!(%local_addr, "BeanAgent web đang lắng nghe");
        web_listener = Some((listener, state));
    }

    let mut telegram = None;
    if let Some(token) = telegram_token {
        let channel = Arc::new(TelegramChannel::new(token, config.telegram.clone()));
        router
            .register_channel(channel.clone())
            .context("đăng ký TelegramChannel thất bại")?;
        telegram = Some(channel);
    }
    router
        .start_outbox_worker()
        .context("không khởi động được worker outbox")?;

    let shutdown = CancellationToken::new();
    let scheduler_shutdown = shutdown.clone();
    let scheduler_task = tokio::spawn(async move {
        scheduler.run(scheduler_shutdown).await;
    });
    let web_shutdown = shutdown.clone();
    let telegram_shutdown = shutdown.clone();
    let telegram_router = router.clone();
    let web_future = async move {
        match web_listener {
            Some((listener, state)) => serve(
                listener,
                build_router(state).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown({
                let shutdown = web_shutdown;
                async move { shutdown.cancelled().await }
            })
            .await
            .context("web server dừng lỗi"),
            None => std::future::pending::<Result<()>>().await,
        }
    };
    let telegram_future = async move {
        match telegram {
            Some(telegram) => telegram.run(telegram_router, telegram_shutdown).await,
            None => std::future::pending::<Result<()>>().await,
        }
    };
    tokio::pin!(web_future);
    tokio::pin!(telegram_future);
    enum FirstExit {
        Signal,
        Web,
        Telegram,
    }
    let (first_exit, result) = tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            if let Err(error) = signal {
                tracing::warn!(%error, "không lắng nghe được Ctrl-C");
            }
            (FirstExit::Signal, Ok(()))
        }
        result = &mut web_future => (FirstExit::Web, result),
        result = &mut telegram_future => (FirstExit::Telegram, result),
    };
    shutdown.cancel();
    match first_exit {
        FirstExit::Signal => {
            if web_enabled {
                let _ = web_future.await;
            }
            if telegram_enabled {
                let _ = telegram_future.await;
            }
        }
        FirstExit::Web => {
            if telegram_enabled {
                let _ = telegram_future.await;
            }
        }
        FirstExit::Telegram => {
            if web_enabled {
                let _ = web_future.await;
            }
        }
    }
    router.shutdown();
    let _ = scheduler_task.await;
    mcp.close().await;
    result
}
