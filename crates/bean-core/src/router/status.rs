//! Quan sát và điều khiển từ adapter: `snapshot`, `cancel`, danh sách agent,
//! registry tool, gọi tool thay mặt (`call_tool_as`).

use super::*;

impl Router {
    /// Đọc snapshot run/confirm hiện tại cho WebSocket `Sync`.
    pub fn snapshot(&self) -> RouterSnapshot {
        let state = match lock(&self.inner.state) {
            Ok(state) => state,
            Err(_) => {
                return RouterSnapshot {
                    running: Vec::new(),
                    pending_confirms: Vec::new(),
                };
            }
        };
        let running = state.queue.running();
        let pending_confirms = ConfirmRegistry::pending(&state);
        RouterSnapshot {
            running,
            pending_confirms,
        }
    }

    /// Báo cáo chuẩn hoá `{status, summary, risks}` cho từng agent con.
    ///
    /// # Vì sao suy ra ở đây, không để UI tự đoán
    ///
    /// `status` chỉ được tính từ **tín hiệu quan sát được** trong `state`:
    /// có run `active` thì `Working`, có confirm chờ thì `AwaitingYou`, còn lại
    /// `Idle`. UI **không** được tự suy luận từ `running`/`pending_confirms` — nếu
    /// để client tự chấm điểm, mỗi client có thể ra một kết luận khác nhau và
    /// bộ lọc RBAC phía server trở nên vô nghĩa.
    ///
    /// `summary` là **văn bản tĩnh theo trạng thái**, không phải dữ liệu thô của
    /// agent: đúng nguyên tắc "Manager không đọc dữ liệu thô của agent con".
    /// `risks` chỉ liệt kê hành động đang chờ người dùng duyệt, vì đó là rủi ro
    /// duy nhất mà HUB được phép nhắc.
    ///
    /// Danh sách role lấy từ `[[roles]]` — cấu hình là nguồn sự thật duy nhất về
    /// "hệ này gồm những ai", nên thêm/bớt agent là một thay đổi cấu hình, không
    /// phải sửa code UI.
    #[must_use]
    pub fn agent_reports(&self) -> Vec<AgentReport> {
        let config = match read_lock(&self.inner.config) {
            Ok(config) => config.clone(),
            Err(_) => return Vec::new(),
        };
        let snapshot = self.snapshot();
        config
            .roles
            .iter()
            .map(|role| {
                let name = role.name.clone();
                let waiting = snapshot
                    .pending_confirms
                    .iter()
                    .filter(|pending| pending.role.as_deref() == Some(name.as_str()))
                    .count();
                let working = snapshot
                    .running
                    .iter()
                    .filter(|run| run.role.as_deref() == Some(name.as_str()))
                    .count();
                // `AwaitingYou` thắng `Working`: việc cần người dùng làm ngay phải
                // hiện trước việc đang tự chạy.
                let (status, summary) = if waiting > 0 {
                    (
                        AgentStatus::AwaitingYou,
                        format!(
                            "{} hành động đang chờ bạn duyệt",
                            waiting.min(MAX_SUMMARY_COUNT)
                        ),
                    )
                } else if working > 0 {
                    (AgentStatus::Working, "đang thực hiện lượt".to_string())
                } else {
                    (AgentStatus::Idle, "không có việc nào đang chạy".to_string())
                };
                let risks = if waiting == 0 {
                    Vec::new()
                } else {
                    snapshot
                        .pending_confirms
                        .iter()
                        .filter(|pending| pending.role.as_deref() == Some(name.as_str()))
                        .map(|pending| pending.prompt.clone())
                        .collect()
                };
                AgentReport {
                    role: name,
                    status,
                    summary,
                    risks,
                    relation: role.relation(),
                }
            })
            .collect()
    }

    /// Huỷ active run của channel/chat; queued run không bị huỷ.
    pub async fn cancel(&self, channel: &str, chat_id: &str) {
        let token = lock(&self.inner.state)
            .ok()
            .and_then(|state| state.queue.cancel_token_for_channel(channel, chat_id));
        if let Some(token) = token {
            token.cancel();
        }
    }

    /// Huỷ đúng run đang active của một session.
    ///
    /// WebSocket gửi `session_id`, nên adapter không được dùng `cancel(channel,
    /// chat_id)` vì nhiều session của cùng user có thể bị huỷ nhầm.
    pub async fn cancel_session(&self, session: SessionId) {
        let token = lock(&self.inner.state)
            .ok()
            .and_then(|state| state.queue.cancel_token_for_session(session));
        if let Some(token) = token {
            token.cancel();
        }
    }

    /// Active run hiện tại, dùng cho test/Sync M9.
    #[must_use]
    pub fn active_run(&self, channel: &str, chat_id: &str) -> Option<RunId> {
        let state = lock(&self.inner.state).ok()?;
        state.queue.active_run_for(channel, chat_id)
    }

    // -----------------------------------------------------------------------
    // M25 — đường MCP server (read-only)
    // -----------------------------------------------------------------------

    /// Registry tool (dùng cho cổng expose MCP — M25).
    #[must_use]
    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.inner.registry
    }

    /// Resolve quyền cho một client MCP (M25).
    ///
    /// # Errors
    /// [`RouterError::StatePoisoned`] nếu không đọc được cấu hình.
    ///
    /// Đây là **cùng** hàm quyết định [`Config::permissions_for`] mà run chat dùng
    /// (M21.3) — không có bản sao logic nào cho đường MCP (ràng buộc `Plan.md` mục 4.3).
    pub fn mcp_permissions(&self, user_id: &str) -> Result<RolePermissions, RouterError> {
        let config = read_lock(&self.inner.config)?;
        Ok(config.permissions_for(user_id))
    }

    /// Gọi tool **trực tiếp** theo danh tính client MCP, không qua vòng lặp agent (M25).
    ///
    /// Vì sao không dùng `submit()`: MCP client gọi một tool cụ thể đã biết tên, không cần
    /// LLM chọn tool, không cần context, không cần confirm. Chạy qua agent loop sẽ tốn
    /// token vô nghĩa và mở thêm bề mặt (system prompt, lịch sử).
    ///
    /// # Errors
    /// * [`RouterError::ToolNotExposed`] — tool không qua được cổng expose
    ///   [`crate::mcp_server::expose_gate`] (tag + `Safe` + RBAC). Client gọi thẳng tên
    ///   tool không thuộc bề mặt MCP sẽ bị chặn ở đây, không phải chỉ ẩn khỏi danh sách.
    /// * [`RouterError::Internal`] — không lấy được workspace/tool.
    ///
    /// # Tham số từ client
    ///
    /// Tham số được **làm sạch ở đây** ([`crate::mcp_server::sanitize_client_args`]),
    /// không phải ở handler: đây là ranh giới duy nhất đi vào tool từ phía ngoài nên
    /// không thể bị bỏ sót nếu sau này thêm một caller mới. Handler chỉ tiếp thụ kết quả.
    pub async fn call_tool_as(
        &self,
        user_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<String, RouterError> {
        let args = crate::mcp_server::sanitize_client_args(&args);
        let permissions = self.mcp_permissions(user_id)?;
        match crate::mcp_server::expose_gate(tool_name, &self.inner.registry, &permissions) {
            crate::mcp_server::ExposeDecision::Allow => {}
            decision => {
                tracing::warn!(
                    user_id,
                    tool = tool_name,
                    ?decision,
                    "từ chối gọi tool qua MCP server"
                );
                return Err(RouterError::ToolNotExposed(tool_name.to_string()));
            }
        }
        let tool = self
            .inner
            .registry
            .get(tool_name)
            .ok_or_else(|| RouterError::ToolNotExposed(tool_name.to_string()))?;
        let workspace = self
            .inner
            .registry
            .workspace_opt()
            .ok_or_else(|| RouterError::Internal("registry chưa gắn workspace".into()))?;
        let ctx = bean_tools::ToolCtx::for_project(
            workspace,
            // MCP không có hội thoại; dùng session 0 làm giá trị trung tính cho
            // trường `session` mà tool chỉ dùng để ghi log/audit.
            SessionId::new(0),
            self.inner.shutdown.clone(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .with_alerts_opt(self.alert_sink());
        let result = tool.call(&ctx, args).await;
        if let Some(log) = &self.inner.audit {
            let mut entry =
                bean_security::entry_now(0, user_id, tool_name, &serde_json::json!({"via": "mcp"}));
            entry.ok = Some(result.is_ok());
            entry.decision = if result.is_ok() { "allow" } else { "deny" };
            entry.decided_by = user_id.to_string();
            entry.error = result.as_ref().err().map(ToString::to_string);
            if let Err(error) = log.record(&entry) {
                tracing::warn!(error = %error, "không ghi được audit MCP");
            }
        }
        result.map_err(|error| RouterError::Internal(error.to_string()))
    }

    pub(super) fn session_busy(&self, session: SessionId) -> bool {
        lock(&self.inner.state).is_ok_and(|state| state.queue.is_busy(session))
    }
}
