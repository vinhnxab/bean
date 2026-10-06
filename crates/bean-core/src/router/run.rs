//! Vòng đời một run: `enqueue`, `spawn`, `execute`, `spawn_reflection`, `finish`
//! (agents.md mục 6 + 10). Hàng đợi theo session nằm ở [`queue`](super::queue).

use super::*;

impl Router {
    pub(super) fn enqueue(&self, queued: QueuedRun) -> Result<(), RouterError> {
        let (start, position) = {
            let mut state = lock(&self.inner.state)?;
            match state.queue.enqueue(&queued) {
                Enqueued::Start => {
                    // "Cho phép tool trong phiên" chỉ tồn tại khi session có run thật.
                    state
                        .policies
                        .entry(queued.session_id)
                        .or_insert_with(|| Arc::new(SessionPolicy::new()));
                    (true, None)
                }
                Enqueued::Queued { position } => (false, Some(position)),
            }
        };
        if let Some(position) = position {
            self.emit(RunEvent::Queued {
                session_id: queued.session_id,
                run_id: queued.run_id.clone(),
                position,
            });
        }
        if start {
            self.spawn(queued);
        }
        Ok(())
    }

    pub(super) fn spawn(&self, queued: QueuedRun) {
        let router = self.clone();
        tokio::spawn(async move {
            router.execute(queued.clone()).await;
            router.finish(&queued.run_id, queued.session_id);
        });
    }

    /// Tên role hiển thị cho một user, dùng để gắn run/confirm về đúng agent.
    ///
    /// `None` khi RBAC tắt (`permissions_for` trả `unrestricted` với role `default`)
    /// — khi đó không có "agent con" nào để quy và HUB sẽ bỏ qua trạng thái theo agent,
    /// chỉ hiện phần Manager. Cố tình **không** trả `no-access`/`default` để tránh rò
    /// tên role nội bộ ra ngoài, và không ép đọc `RwLock` khi đang giữ khoá khác.
    pub(super) fn current_role(&self, incoming: &Incoming) -> Option<String> {
        let config = read_lock(&self.inner.config).ok()?;
        if !config.rbac_enabled() {
            return None;
        }
        let role = config.permissions_for(&incoming.user_id).role;
        if role == NO_ACCESS_ROLE || role == "default" {
            return None;
        }
        Some(role)
    }

    pub(super) async fn execute(&self, queued: QueuedRun) {
        let config = match read_lock(&self.inner.config) {
            Ok(config) => config.clone(),
            Err(error) => {
                self.emit_error(&queued, "router_state", &error.to_string());
                return;
            }
        };
        let policy = if queued.background_allowed_tools.is_some() {
            // Scheduler không dùng allow-in-session của một phiên tương tác.
            Arc::new(SessionPolicy::new())
        } else {
            lock(&self.inner.state)
                .ok()
                .and_then(|state| state.policies.get(&queued.session_id).cloned())
                .unwrap_or_else(|| Arc::new(SessionPolicy::new()))
        };
        let audit_channel = if queued.background_allowed_tools.is_some() {
            "scheduler"
        } else {
            &queued.incoming.channel
        };
        let io = Arc::new(RouterIo::new(
            Arc::downgrade(&self.inner),
            queued.session_id,
            queued.run_id.clone(),
            queued.incoming.user_id.clone(),
            queued.cancel.clone(),
            queued.background_allowed_tools.clone(),
        ));
        let skills_index = match self.inner.skills.index() {
            Ok(index) => index,
            Err(error) => {
                self.emit_error(&queued, "router_state", &error.to_string());
                return;
            }
        };
        // (M21.3) **Điểm quyết định RBAC duy nhất.** Router resolve role → `RolePermissions`
        // một lần mỗi run rồi truyền struct tuần tự hoá được xuống agent loop. Mọi lần lọc
        // tool (payload và tầng thực thi) đều dùng **cùng** struct này nên không thể lệch nhau,
        // và không logic RBAC nào nằm rải trong tool/role (ràng buộc `Plan.md` mục 4.3).
        let permissions: RolePermissions = config.permissions_for(&queued.incoming.user_id);
        // (M21.1) Project của lượt; `execute` chỉ chạy sau khi `submit` đã validate qua
        // `resolve_project`, nên ở đây dùng `unwrap_or(default)` là lưới an toàn.
        let project = queued
            .incoming
            .project
            .clone()
            .unwrap_or_else(|| bean_types::config::DEFAULT_PROJECT.to_string());
        tracing::debug!(
            run_id = %queued.run_id,
            user_id = %queued.incoming.user_id,
            role = %permissions.role,
            project = %project,
            "resolve quyền cho run"
        );
        let result = run_turn_outcome(RunTurnArgs {
            store: self.inner.store.as_ref(),
            registry: self.inner.registry.as_ref(),
            llm: self.inner.llm.as_ref(),
            config: &config,
            session: queued.session_id,
            user_text: queued.incoming.text.clone(),
            io,
            cancel: queued.cancel.clone(),
            session_policy: Some(policy),
            audit: self.inner.audit.clone(),
            channel: audit_channel,
            skills_index: &skills_index,
            permissions: &permissions,
            project: &project,
            alerts: self.alert_sink(),
        })
        .await;
        match result {
            Ok(outcome) => {
                let should_reflect = outcome.ended == crate::agent::EndReason::Final
                    && outcome.tool_call_count
                        >= usize::try_from(config.learning.min_tool_calls).unwrap_or(usize::MAX);
                self.emit(RunEvent::Final {
                    session_id: queued.session_id,
                    run_id: queued.run_id.clone(),
                    text: outcome.text.clone(),
                    message_id: outcome.message_id,
                });
                if should_reflect {
                    let catalog = self.inner.skills.catalog().ok();
                    if let Some(reservation) =
                        self.inner.learning_gate.reserve(&config, catalog.as_ref())
                    {
                        self.spawn_reflection(queued, config, outcome, reservation);
                    }
                }
            }
            Err(error) => {
                let code = error.code();
                self.emit_error(&queued, code, &error.to_string());
            }
        }
    }

    pub(super) fn spawn_reflection(
        &self,
        queued: QueuedRun,
        config: Config,
        outcome: RunOutcome,
        reservation: chrono::DateTime<chrono::Utc>,
    ) {
        let Some(catalog) = self.inner.skills.catalog().ok() else {
            self.inner.learning_gate.release(reservation);
            return;
        };
        let router = self.clone();
        let shutdown = self.inner.shutdown.clone();
        let created_at = reservation.to_rfc3339();
        tokio::spawn(async move {
            let result = reflect(ReflectionArgs {
                store: router.inner.store.as_ref(),
                llm: router.inner.llm.as_ref(),
                config: &config,
                catalog: &catalog,
                transcript: &outcome.transcript,
                loaded_skills: &outcome.loaded_skills,
                session: queued.session_id,
                channel: &queued.incoming.channel,
                chat_id: &queued.incoming.chat_id,
                created_at: &created_at,
                cancel: shutdown,
            })
            .await;
            match result {
                Ok(Some(draft)) => {
                    let verb = match draft.kind {
                        bean_skills::SkillDraftKind::New => "mới",
                        bean_skills::SkillDraftKind::Update => "sửa",
                    };
                    let text = format!(
                        "Đề xuất skill {verb} `{}` đã sẵn sàng để duyệt.\nLý do: {}\nDuyệt: /approve {} · Bỏ: /reject {}",
                        draft.name, draft.reason, draft.id, draft.id
                    );
                    match router
                        .inner
                        .store
                        .append(
                            queued.session_id,
                            bean_types::Message::assistant(Some(text.clone()), Vec::new()),
                        )
                        .await
                    {
                        Ok(message_id) => {
                            let outbound = bean_types::Outbound {
                                session_id: queued.session_id,
                                message_id,
                                text,
                                kind: bean_types::OutboundKind::Notification,
                                action: Some(bean_types::OutboundAction::SkillDraft {
                                    id: draft.id,
                                    name: draft.name,
                                    actor: queued.incoming.user_id.clone(),
                                }),
                            };
                            if let Err(error) = router
                                .notify(
                                    &queued.incoming.channel,
                                    &queued.incoming.chat_id,
                                    outbound,
                                )
                                .await
                            {
                                tracing::warn!(error = %error, "gửi thông báo skill nháp thất bại");
                            }
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "không lưu thông báo skill nháp");
                        }
                    }
                }
                Ok(None) => router.inner.learning_gate.release(reservation),
                Err(error) => {
                    tracing::warn!(error = %error, "reflection M15 thất bại; run chính vẫn hoàn tất");
                    router.inner.learning_gate.release(reservation);
                }
            }
        });
    }

    pub(super) fn finish(&self, run_id: &RunId, session_id: SessionId) {
        let (next, positions, stale) = {
            let mut state = match lock(&self.inner.state) {
                Ok(state) => state,
                Err(_) => return,
            };
            // Confirm của run này trở nên vô nghĩa ⇒ gỡ và báo `Denied` cho UI.
            let stale = ConfirmRegistry::drop_for_run(&mut state, run_id);
            let Some(advanced) =
                state
                    .queue
                    .advance(run_id, session_id, self.inner.shutdown.is_cancelled())
            else {
                return;
            };
            (advanced.next, advanced.positions, stale)
        };
        for (confirm_id, confirm_session, confirm_run) in stale {
            self.emit(RunEvent::ConfirmResolved {
                session_id: confirm_session,
                run_id: confirm_run,
                confirm_id,
                outcome: ConfirmOutcome::Denied,
            });
        }
        for (run_id, position) in positions {
            self.emit(RunEvent::Queued {
                session_id,
                run_id,
                position,
            });
        }
        if let Some(next) = next {
            self.spawn(next);
        }
    }
}
