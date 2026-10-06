//! Tiếp nhận input: khởi tạo, đăng ký kênh, `submit`/`submit_scheduled`,
//! authorize, resolve session/project (`admit` tách ra từ `Router`).

use super::*;

impl Router {
    /// Tạo Router với timeout chuẩn 300 giây.
    #[must_use]
    pub fn new(deps: RouterDeps) -> Self {
        Self::with_options(deps, RouterOptions::default())
    }

    /// Tạo Router với options tường minh (test có thể rút ngắn thời gian).
    #[must_use]
    pub fn with_options(deps: RouterDeps, options: RouterOptions) -> Self {
        let RouterDeps {
            config,
            store,
            registry,
            llm,
            audit,
            skills_index,
            skills,
        } = deps;
        let (events, _) = broadcast::channel(options.event_capacity.max(1));
        Self {
            inner: Arc::new(RouterInner {
                config: RwLock::new(config),
                store,
                registry,
                llm,
                audit,
                skills: SkillCoordinator::new(skills, skills_index),
                learning_gate: LearningGate::default(),
                options,
                events,
                channels: RwLock::new(HashMap::new()),
                state: Mutex::new(RouterState::default()),
                submission: AsyncMutex::new(()),
                outbox_started: AtomicBool::new(false),
                shutdown: CancellationToken::new(),
            }),
        }
    }

    /// Đăng ký adapter gửi outbound.
    pub fn register_channel(&self, channel: Arc<dyn Channel>) -> Result<(), RouterError> {
        let name = channel.name().to_string();
        let mut channels = write_lock(&self.inner.channels)?;
        if channels.contains_key(&name) {
            return Err(RouterError::DuplicateChannel(name));
        }
        channels.insert(name, channel);
        Ok(())
    }

    /// Subscribe trực tiếp vào broadcast.
    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<RunEvent> {
        self.inner.events.subscribe()
    }

    /// Nhận event; `Lagged` được log rồi tiếp tục, không làm run chết.
    pub async fn recv_event(
        &self,
        receiver: &mut broadcast::Receiver<RunEvent>,
    ) -> Option<RunEvent> {
        loop {
            match receiver.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "subscriber Router bị lag; tiếp tục stream");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Dừng worker outbox và huỷ các run đang active.
    ///
    /// Đây là shutdown của process, khác với `cancel` của người dùng: mọi token
    /// active đều được huỷ để tool/confirm không giữ tiến trình sống sau SIGTERM.
    pub fn shutdown(&self) {
        self.inner.shutdown.cancel();
        let tokens = lock(&self.inner.state)
            .map(|state| state.queue.all_cancel_tokens())
            .unwrap_or_default();
        for token in tokens {
            token.cancel();
        }
    }

    /// Cổng vào chung cho mọi lượt mới: phân quyền → validate project → xếp khoá
    /// submit → cấp `run_id` → resolve session.
    ///
    /// # Vì sao tách riêng
    ///
    /// `submit` và `submit_scheduled` trước đây lặp lại **đúng** chuỗi này; chỉ khác
    /// ở dòng cuối cùng (slash command hay `background_allowed_tools`). Hai bản có
    /// thể lệch nhau khi thêm một bước kiểm tra mới và chỉ sửa một trong hai.
    ///
    /// # Errors
    /// [`RouterError`] từ phân quyền, project lạ, sinh `run_id` hoặc store.
    pub(super) async fn admit(&self, incoming: &Incoming) -> Result<Admission<'_>, RouterError> {
        self.authorize(incoming)?;
        // (M21.1) Validate project trước khi xếp hàng — lỗi trả về cho adapter, không
        // phải giữa run.
        {
            let config = read_lock(&self.inner.config)?;
            self.resolve_project(incoming, &config)?;
        }
        let guard = self.inner.submission.lock().await;
        let run_id = new_run_id()?;
        let session = self.resolve_session(incoming).await?;
        Ok(Admission {
            _guard: guard,
            run_id,
            session,
        })
    }

    /// Submit trả `RunId` ngay; run LLM được spawn nền.
    pub async fn submit(&self, incoming: Incoming) -> Result<RunId, RouterError> {
        let admission = self.admit(&incoming).await?;
        let run_id = admission.run_id;
        let text = incoming.text.trim();
        if text.starts_with('/') {
            self.handle_command(&incoming, admission.session, run_id.clone(), text)
                .await?;
            return Ok(run_id);
        }

        self.enqueue(QueuedRun {
            run_id: run_id.clone(),
            session_id: admission.session,
            role: self.current_role(&incoming),
            incoming,
            cancel: CancellationToken::new(),
            background_allowed_tools: None,
        })?;
        Ok(run_id)
    }

    /// Submit một prompt tự động cho session đã được scheduler xác nhận.
    ///
    /// Khác với run interactive, mọi tool Confirm/Dangerous sẽ được RouterIo
    /// cho chạy nếu nằm trong `allowed_tools`, nếu không sẽ bị từ chối ngay.
    pub async fn submit_scheduled(
        &self,
        incoming: Incoming,
        allowed_tools: Vec<String>,
    ) -> Result<RunId, RouterError> {
        let admission = self.admit(&incoming).await?;
        let run_id = admission.run_id;
        self.enqueue(QueuedRun {
            run_id: run_id.clone(),
            session_id: admission.session,
            role: self.current_role(&incoming),
            incoming,
            cancel: CancellationToken::new(),
            background_allowed_tools: Some(Arc::new(allowed_tools.into_iter().collect())),
        })?;
        Ok(run_id)
    }

    pub(super) fn authorize(&self, incoming: &Incoming) -> Result<(), RouterError> {
        let config = read_lock(&self.inner.config)?;
        if !config
            .agent
            .allowed_users
            .iter()
            .any(|allowed| allowed == &incoming.user_id)
        {
            tracing::warn!(user_id = %incoming.user_id, "từ chối user ngoài allowlist");
            return Err(RouterError::Forbidden(incoming.user_id.clone()));
        }
        let prefix = format!("{}:", incoming.channel);
        if !incoming.user_id.starts_with(&prefix) {
            return Err(RouterError::InvalidIdentity {
                channel: incoming.channel.clone(),
            });
        }
        Ok(())
    }

    /// Chọn project profile cho lượt (M21.1).
    ///
    /// Tên lạ ⇒ `RouterError::InvalidProject` (fail-closed: không rơi về project khác, vì
    /// làm vậy sẽ cho phiên đọc/ghi nhầm `MEMORY.md` của project khác).
    pub(super) fn resolve_project(
        &self,
        incoming: &Incoming,
        config: &Config,
    ) -> Result<String, RouterError> {
        let requested = incoming
            .project
            .as_deref()
            .unwrap_or(bean_types::config::DEFAULT_PROJECT);
        if config.project_workspace(requested).is_some() {
            Ok(requested.to_string())
        } else {
            Err(RouterError::InvalidProject(requested.to_string()))
        }
    }

    pub(super) async fn resolve_session(
        &self,
        incoming: &Incoming,
    ) -> Result<SessionId, RouterError> {
        if let Some(session) = incoming.session_id {
            let info = self.inner.store.session_info(session).await?;
            let owned = info.is_some_and(|info| {
                !info.archived
                    && info.channel == incoming.channel
                    && info.chat_id == incoming.chat_id
                    && info.user_id == incoming.user_id
            });
            if !owned {
                return Err(RouterError::InvalidSession);
            }
            return Ok(session);
        }
        self.inner
            .store
            .ensure_session_for_user(&incoming.channel, &incoming.chat_id, &incoming.user_id, "")
            .await
            .map_err(RouterError::Store)
    }
}
