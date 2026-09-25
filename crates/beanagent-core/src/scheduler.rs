//! M13 scheduler: cron theo múi giờ người dùng, chạy nền qua Router và outbox bền vững.

use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beanagent_memory::{NewScheduledTask, ScheduledTask, Store, StoreError};
use beanagent_tools::{Tool, ToolCtx, ToolError, TypedTool};
use beanagent_types::{Outbound, OutboundKind, Risk, RunEvent, SessionId};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::router::{Incoming, Router, RouterError};

/// Lỗi scheduler không làm chết worker tick.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// Tên timezone IANA không hợp lệ.
    #[error("timezone không hợp lệ: {0}")]
    InvalidTimezone(String),
    /// Biểu thức cron không hợp lệ.
    #[error("cron không hợp lệ: {0}")]
    InvalidCron(String),
    /// Không tìm thấy occurrence kế tiếp.
    #[error("không tìm thấy lần chạy kế tiếp cho cron")]
    NoNextOccurrence,
    /// Lỗi store.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Lỗi Router.
    #[error(transparent)]
    Router(#[from] RouterError),
    /// Broadcast của Router đã đóng trước khi nhận kết quả.
    #[error("Router event stream đã đóng")]
    EventStreamClosed,
}

/// Nguồn thời gian để test deterministic; production dùng [`SystemClock`].
pub trait Clock: Send + Sync {
    /// Thời điểm UTC hiện tại.
    fn now(&self) -> DateTime<Utc>;
}

/// Clock production, lấy UTC từ hệ điều hành.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Clock giả có thể đặt/đẩy thời gian trong test M13.
#[derive(Debug)]
pub struct FixedClock {
    now: Mutex<DateTime<Utc>>,
}

impl FixedClock {
    /// Tạo clock tại một thời điểm UTC cụ thể.
    #[must_use]
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    /// Đặt thời điểm hiện tại.
    pub fn set(&self, now: DateTime<Utc>) {
        match self.now.lock() {
            Ok(mut current) => *current = now,
            Err(poisoned) => *poisoned.into_inner() = now,
        }
    }

    /// Đẩy clock thêm một khoảng thời gian.
    pub fn advance(&self, amount: Duration) {
        let delta = chrono::Duration::from_std(amount).unwrap_or(chrono::Duration::MAX);
        match self.now.lock() {
            Ok(mut current) => *current += delta,
            Err(poisoned) => *poisoned.into_inner() += delta,
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        match self.now.lock() {
            Ok(current) => *current,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

/// Parse cron theo timezone IANA rồi trả occurrence kế tiếp ở UTC.
pub fn next_run_after(
    cron: &str,
    timezone: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, SchedulerError> {
    let tz = Tz::from_str(timezone)
        .map_err(|_| SchedulerError::InvalidTimezone(timezone.to_string()))?;
    let pattern =
        Cron::from_str(cron).map_err(|error| SchedulerError::InvalidCron(error.to_string()))?;
    let local_now = now.with_timezone(&tz);
    let next = pattern
        .find_next_occurrence(&local_now, false)
        .map_err(|_| SchedulerError::NoNextOccurrence)?;
    Ok(next.with_timezone(&Utc))
}
const DEFAULT_TICK: Duration = Duration::from_secs(30);
const DEFAULT_BATCH_SIZE: usize = 100;

/// Worker tick 30 giây của M13.
#[derive(Clone)]
pub struct Scheduler {
    store: Arc<dyn Store>,
    router: Arc<Router>,
    clock: Arc<dyn Clock>,
    timezone: String,
    tick: Duration,
    batch_size: usize,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("timezone", &self.timezone)
            .field("tick", &self.tick)
            .field("batch_size", &self.batch_size)
            .finish_non_exhaustive()
    }
}

impl Scheduler {
    /// Tạo scheduler với clock production và chu kỳ 30 giây.
    pub fn new(
        store: Arc<dyn Store>,
        router: Arc<Router>,
        timezone: impl Into<String>,
    ) -> Result<Self, SchedulerError> {
        Self::with_clock(store, router, timezone, Arc::new(SystemClock))
    }

    /// Tạo scheduler với clock inject được, dùng cho test hoặc runtime đặc biệt.
    pub fn with_clock(
        store: Arc<dyn Store>,
        router: Arc<Router>,
        timezone: impl Into<String>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, SchedulerError> {
        let timezone = timezone.into();
        Tz::from_str(&timezone).map_err(|_| SchedulerError::InvalidTimezone(timezone.clone()))?;
        Ok(Self {
            store,
            router,
            clock,
            timezone,
            tick: DEFAULT_TICK,
            batch_size: DEFAULT_BATCH_SIZE,
        })
    }

    /// Đổi chu kỳ tick; 0 được đưa về 1ms.
    #[must_use]
    pub fn with_tick(mut self, tick: Duration) -> Self {
        self.tick = tick.max(Duration::from_millis(1));
        self
    }

    /// Đặt số task xử lý tối đa trong một tick.
    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size.max(1);
        self
    }

    /// Đăng ký ba tool quản lý tác vụ định kỳ.
    #[must_use]
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        schedule_tools(
            self.store.clone(),
            self.timezone.clone(),
            self.clock.clone(),
        )
    }

    /// Chạy vòng tick cho tới khi `shutdown` được huỷ.
    pub async fn run(self: Arc<Self>, shutdown: CancellationToken) {
        let mut ticker = tokio::time::interval(self.tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = ticker.tick() => {
                    if let Err(error) = self.run_once().await {
                        tracing::warn!(error = %error, "scheduler tick lỗi");
                    }
                }
            }
        }
    }

    /// Xử lý một tick; trả số task thực sự được submit thành công.
    pub async fn run_once(&self) -> Result<usize, SchedulerError> {
        let now = self.clock.now();
        let now_text = now.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        let tasks = self.store.due_tasks(&now_text, self.batch_size).await?;
        let mut submitted = 0;
        for task in tasks {
            let task_id = task.id;
            match self.process_task(task, now).await {
                Ok(true) => submitted += 1,
                Ok(false) => {}
                Err(error) => {
                    let _ = self.store.set_task_status(task_id, "error").await;
                    tracing::warn!(task_id, error = %error, "scheduler không xử lý được task");
                }
            }
        }
        Ok(submitted)
    }

    async fn process_task(
        &self,
        task: ScheduledTask,
        now: DateTime<Utc>,
    ) -> Result<bool, SchedulerError> {
        let next = next_run_after(&task.cron, &self.timezone, now)?;
        let now_text = now.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        // Tick 30 giây có thể đến trễ vài giây; chỉ coi là lỡ hạn khi vượt grace
        // để không bỏ sót occurrence chỉ vì ticker không gọi đúng mili-giây.
        let grace = chrono::Duration::from_std(self.tick.saturating_mul(2))
            .unwrap_or(chrono::Duration::MAX);
        let overdue = task
            .next_run
            .parse::<DateTime<Utc>>()
            .map(|scheduled| now - scheduled > grace)
            .unwrap_or(task.next_run.as_str() < now_text.as_str());
        let initial_status = if overdue { "skipped" } else { "running" };
        let Some(claimed) = self
            .store
            .claim_task(
                task.id,
                &task.next_run,
                &next.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
                &now_text,
                initial_status,
            )
            .await?
        else {
            return Ok(false);
        };

        if overdue || claimed.session_id.is_none() {
            if claimed.session_id.is_none() {
                let _ = self.store.set_task_status(task.id, "skipped").await;
            }
            return Ok(false);
        }
        let session = claimed.session_id.unwrap_or_else(|| SessionId::new(0));
        let Some(info) = self.store.session_info(session).await? else {
            let _ = self.store.set_task_status(task.id, "error").await;
            return Ok(false);
        };
        if info.archived
            || info.user_id.is_empty()
            || info.channel != claimed.channel
            || info.chat_id != claimed.chat_id
        {
            let _ = self.store.set_task_status(task.id, "error").await;
            return Ok(false);
        }

        let incoming = Incoming::new(
            claimed.channel.clone(),
            claimed.chat_id.clone(),
            info.user_id,
            claimed.prompt.clone(),
        )
        .with_session(session);
        let mut events = self.router.events();
        let run_id = match self
            .router
            .submit_scheduled(incoming, claimed.allowed_tools.clone())
            .await
        {
            Ok(run_id) => run_id,
            Err(error) => {
                let _ = self.store.set_task_status(task.id, "error").await;
                return Err(SchedulerError::Router(error));
            }
        };

        loop {
            let event = self
                .router
                .recv_event(&mut events)
                .await
                .ok_or(SchedulerError::EventStreamClosed)?;
            match event {
                RunEvent::Final {
                    run_id: finished,
                    text,
                    message_id,
                    ..
                } if finished == run_id => {
                    let message_id = match message_id {
                        Some(id) => id,
                        None => {
                            self.store
                                .append(
                                    session,
                                    beanagent_types::Message::assistant(
                                        Some(text.clone()),
                                        Vec::new(),
                                    ),
                                )
                                .await?
                        }
                    };
                    let outbound = Outbound {
                        session_id: session,
                        message_id,
                        text,
                        kind: OutboundKind::Notification,
                        action: None,
                    };
                    self.router
                        .notify(&claimed.channel, &claimed.chat_id, outbound)
                        .await?;
                    let _ = self.store.set_task_status(task.id, "success").await;
                    return Ok(true);
                }
                RunEvent::Error { run_id: failed, .. } if failed == run_id => {
                    let _ = self.store.set_task_status(task.id, "error").await;
                    return Ok(false);
                }
                _ => {}
            }
        }
    }
}

/// Tham số tool `schedule_task`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScheduleTaskParams {
    /// Cron 5 trường, hiểu theo timezone của agent, ví dụ `0 7 * * *`.
    cron: String,
    /// Prompt agent sẽ chạy khi task đến hạn.
    prompt: String,
    /// Tên tool được phép chạy tự động; để trống nghĩa là không tool nào.
    #[serde(default)]
    allowed_tools: Vec<String>,
}

/// Tham số tool `list_tasks`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListTasksParams {}

/// Tham số tool `cancel_task`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CancelTaskParams {
    /// ID task cần huỷ.
    id: u64,
}

/// Tạo tool schedule/list/cancel dùng chung store và timezone.
#[must_use]
pub fn schedule_tools(
    store: Arc<dyn Store>,
    timezone: String,
    clock: Arc<dyn Clock>,
) -> Vec<Arc<dyn Tool>> {
    let schedule_store = store.clone();
    let schedule_timezone = timezone.clone();
    let schedule_clock = clock.clone();
    let list_store = store.clone();
    let cancel_store = store;
    vec![
        Arc::new(TypedTool::new(
            "schedule_task",
            Risk::Confirm,
            move |ctx: &ToolCtx, params: ScheduleTaskParams| {
                let store = schedule_store.clone();
                let timezone = schedule_timezone.clone();
                let clock = schedule_clock.clone();
                let session = ctx.session;
                async move {
                    let info = store
                        .session_info(session)
                        .await
                        .map_err(|error| ToolError::Internal(error.to_string()))?
                        .ok_or_else(|| ToolError::InvalidArgs("session không tồn tại".into()))?;
                    if info.archived {
                        return Err(ToolError::InvalidArgs("session đã archive".into()));
                    }
                    let cron = params.cron.trim().to_string();
                    let prompt = params.prompt.trim().to_string();
                    if cron.is_empty() || prompt.is_empty() {
                        return Err(ToolError::InvalidArgs(
                            "cron và prompt không được để trống".into(),
                        ));
                    }
                    let next = next_run_after(&cron, &timezone, clock.now())
                        .map_err(|error| ToolError::InvalidArgs(error.to_string()))?;
                    let allowed_tools = params
                        .allowed_tools
                        .into_iter()
                        .map(|tool| tool.trim().to_string())
                        .filter(|tool| !tool.is_empty())
                        .collect();
                    let task = store
                        .create_task(NewScheduledTask {
                            cron,
                            prompt,
                            session_id: Some(session),
                            channel: info.channel,
                            chat_id: info.chat_id,
                            allowed_tools,
                            next_run: next.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
                            enabled: true,
                        })
                        .await
                        .map_err(|error| ToolError::Internal(error.to_string()))?;
                    Ok(format!(
                        "Đã tạo task #{}; lần chạy kế tiếp: {}",
                        task.id, task.next_run
                    ))
                }
            },
        )),
        Arc::new(TypedTool::new(
            "list_tasks",
            Risk::Safe,
            move |ctx: &ToolCtx, _params: ListTasksParams| {
                let store = list_store.clone();
                let session = ctx.session;
                async move {
                    let tasks = store
                        .list_tasks_for_session(session)
                        .await
                        .map_err(|error| ToolError::Internal(error.to_string()))?;
                    serde_json::to_string_pretty(&tasks)
                        .map_err(|error| ToolError::Internal(error.to_string()))
                }
            },
        )),
        Arc::new(TypedTool::new(
            "cancel_task",
            Risk::Confirm,
            move |ctx: &ToolCtx, params: CancelTaskParams| {
                let store = cancel_store.clone();
                let session = ctx.session;
                async move {
                    let deleted = store
                        .delete_task_for_session(params.id, session)
                        .await
                        .map_err(|error| ToolError::Internal(error.to_string()))?;
                    if deleted {
                        Ok(format!("Đã huỷ task #{}.", params.id))
                    } else {
                        Err(ToolError::NotFound(format!("task #{}", params.id)))
                    }
                }
            },
        )),
    ]
}
