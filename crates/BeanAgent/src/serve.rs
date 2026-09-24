//! `BeanAgent serve` — khởi động web server M9 qua Router.
//!
//! Telegram (M12), scheduler (M13) và các tính năng milestone sau chưa được bật ở đây.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::serve;
use beanagent_core::{Router, RouterDeps};
use beanagent_memory::{SqliteStore, Store};
use beanagent_skills::SkillCatalog;
use beanagent_types::Config;
use beanagent_web::{AuthService, WebChannel, WebState, build_router};
use tokio::net::TcpListener;

use crate::chat;
use crate::cli::{ChatArgs, ServeArgs};

/// Chạy web server và Router với provider được cấu hình.
pub async fn run(args: &ServeArgs, config_path: Option<&Path>) -> Result<()> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    if !config.web.enabled {
        bail!("web.enabled = false; M9 `serve` hiện chỉ cung cấp web server");
    }
    if !config.web.bind.ip().is_loopback() {
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
        web_enabled = true,
        bind = %config.web.bind,
        "khởi động BeanAgent web server"
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
    // Nạp auth trước khi dựng provider/bind socket: web không được chạy fail-open.
    let store_dyn: Arc<dyn Store> = store.clone();
    let auth = AuthService::load(&config, store_dyn.clone()).context(
        "web chưa được bật: chạy `BeanAgent auth set-password` trước (auth.toml phải là argon2id, 0600)",
    )?;

    let chat_args = ChatArgs {
        fake_llm: args.fake_llm.clone(),
        workspace: None,
    };
    let (provider, web_search_api_key) = chat::build_provider(&chat_args, &config)?;
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
    let registry = Arc::new(chat::build_registry(
        &config,
        store.clone(),
        skills.clone(),
        web_search_api_key,
    )?);
    let workspace = registry.workspace_opt();
    let audit = chat::build_audit(&config);
    let router = Arc::new(Router::new(RouterDeps {
        config: config.clone(),
        store: store_dyn.clone(),
        registry,
        llm: provider,
        audit: audit.clone(),
        skills_index,
    }));
    router
        .start_outbox_worker()
        .context("không khởi động được worker outbox")?;

    let state = WebState::new(
        config.clone(),
        store_dyn,
        router.clone(),
        auth,
        audit,
        skills,
        workspace,
    )?;
    let channel = Arc::new(WebChannel::new(state.notifications.clone()));
    router
        .register_channel(channel)
        .context("đăng ký WebChannel thất bại")?;

    let app = build_router(state);
    let listener = TcpListener::bind(config.web.bind)
        .await
        .with_context(|| format!("không bind được {}", config.web.bind))?;
    let local_addr = listener
        .local_addr()
        .context("đọc địa chỉ listener thất bại")?;
    tracing::info!(%local_addr, "BeanAgent web đang lắng nghe");

    let shutdown_router = router.clone();
    serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("đã nhận Ctrl-C; đang đóng web server");
            shutdown_router.shutdown();
        }
    })
    .await
    .context("web server dừng lỗi")?;
    router.shutdown();
    Ok(())
}
