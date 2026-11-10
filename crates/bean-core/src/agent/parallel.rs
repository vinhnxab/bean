//! Phân loại và thực thi **nhóm tool song song** (lộ trình mục 5a).
//!
//! Tách khỏi `run.rs` để giữ trần dòng module (test kiến trúc) và theo đúng
//! trách nhiệm: `run.rs` điều phối vòng lặp, module này quyết định call nào
//! được chạy đồng thời và chạy nhóm đó an toàn.

use super::*;

impl<'a> Agent<'a> {
    /// Chạy **một nhóm** tool `Safe` song song: mỗi call là một task độc lập,
    /// chặn trần [`MAX_PARALLEL_TOOL_CALLS`] bằng semaphore.
    ///
    /// Lỗi từ task (chỉ có thể là lỗi ghi transcript vào store) được lan truyền sau
    /// khi **tất cả** task đã kết thúc — khác vòng tuần tự (dừng ngay), cách này cho
    /// các call còn lại cơ hội ghi kết quả xong để cặp tool_use/tool_result không hỏng.
    pub(super) async fn run_tool_group(
        &self,
        turn: &Turn<'_>,
        state: &TurnState,
        session_policy: &SessionPolicy,
        calls: &[&ToolCall],
    ) -> Result<(), AgentError> {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_PARALLEL_TOOL_CALLS));
        let tasks = calls.iter().map(|call| {
            let semaphore = Arc::clone(&semaphore);
            async move {
                let _permit = semaphore.acquire().await.map_err(|_| {
                    AgentError::Llm("nhóm tool song song đã đóng bất thường".into())
                })?;
                self.run_tool_call(turn, state, session_policy, call).await
            }
        });
        let results = futures_util::future::join_all(tasks).await;
        results
            .into_iter()
            .collect::<Result<Vec<()>, AgentError>>()?;
        Ok(())
    }
}

/// Một đoạn tool call của cùng một lượt model, đã phân loại để thực thi.
#[derive(Debug)]
pub(super) enum ToolSegment<'a> {
    /// Một call chạy tuần tự (mọi tool `Confirm`/`Dangerous`, hoặc nhóm song
    /// song bị tắt).
    Single(&'a ToolCall),
    /// Nhiều call `Safe` **liên tiếp** — chạy đồng thời với trần
    /// [`MAX_PARALLEL_TOOL_CALLS`].
    Parallel(Vec<&'a ToolCall>),
}

/// Phân loại danh sách tool call của một lượt model thành các đoạn tuần tự/song song.
///
/// Nhóm tối đa là các call `Safe` **liên tiếp** trong response: giữ nguyên thứ tự
/// tương đối giữa các call, không xen kẽ một call `Confirm` vào giữa nhóm đọc —
/// model có quyền giả định call trước đã chạy xong khi viết call sau, nên chỉ ghép
/// những call đứng cạnh nhau. Mức rủi ro tính theo `tool.risk(&args)` — cùng quy tắc
/// với `run_tool_call`, không có nguồn sự thật thứ hai.
#[must_use]
pub(super) fn partition_tool_calls<'a>(
    calls: &'a [ToolCall],
    registry: &bean_tools::ToolRegistry,
    parallel_enabled: bool,
) -> Vec<ToolSegment<'a>> {
    let risk_of = |call: &ToolCall| {
        registry
            .get(&call.name)
            .map_or(bean_types::Risk::Safe, |tool| tool.risk(&call.args))
    };
    let mut segments: Vec<ToolSegment<'a>> = Vec::new();
    let mut index = 0;
    while index < calls.len() {
        if parallel_enabled && risk_of(&calls[index]) == bean_types::Risk::Safe {
            let start = index;
            index += 1;
            while index < calls.len() && risk_of(&calls[index]) == bean_types::Risk::Safe {
                index += 1;
            }
            let group: Vec<&ToolCall> = calls[start..index].iter().collect();
            if group.len() == 1 {
                segments.push(ToolSegment::Single(group[0]));
            } else {
                segments.push(ToolSegment::Parallel(group));
            }
        } else {
            segments.push(ToolSegment::Single(&calls[index]));
            index += 1;
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// Tool stub cho test phân nhóm: mức rủi ro phụ thuộc tham số `dangerous`.
    struct PartitionTool;

    #[async_trait::async_trait]
    impl bean_tools::Tool for PartitionTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new("pt", "stub", serde_json::json!({"type": "object"}))
        }
        fn risk(&self, args: &serde_json::Value) -> Risk {
            if args.get("dangerous") == Some(&serde_json::Value::Bool(true)) {
                Risk::Confirm
            } else {
                Risk::Safe
            }
        }
        async fn call(
            &self,
            _ctx: &bean_tools::ToolCtx,
            _args: serde_json::Value,
        ) -> Result<String, bean_tools::ToolError> {
            Ok("ok".into())
        }
    }

    impl std::fmt::Debug for PartitionTool {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("PartitionTool")
        }
    }

    fn partition_registry() -> bean_tools::ToolRegistry {
        let dir = tempfile::tempdir().unwrap();
        // TempDir cố ý leak: chỉ phục vụ test này, process kết thúc sẽ dọn.
        let ws = std::sync::Arc::new(
            bean_security::CapWorkspace::open(
                std::boxed::Box::leak(Box::new(dir)).path().to_path_buf(),
            )
            .unwrap(),
        );
        let mut reg = bean_tools::ToolRegistry::with_workspace(ws);
        reg.register(std::sync::Arc::new(PartitionTool)).unwrap();
        reg
    }

    fn call_named(id: &str, args: serde_json::Value) -> ToolCall {
        ToolCall::new(id, "pt", args)
    }

    fn segment_kinds(segments: &[ToolSegment<'_>]) -> Vec<&'static str> {
        segments
            .iter()
            .map(|s| match s {
                ToolSegment::Single(_) => "single",
                ToolSegment::Parallel(_) => "parallel",
            })
            .collect()
    }

    #[test]
    fn partition_groups_consecutive_safe_calls() {
        let reg = partition_registry();
        let calls = vec![
            call_named("c1", serde_json::json!({})),
            call_named("c2", serde_json::json!({})),
            call_named("c3", serde_json::json!({})),
        ];
        let segments = partition_tool_calls(&calls, &reg, true);
        assert_eq!(segment_kinds(&segments), vec!["parallel"]);
        let ids: Vec<&str> = match &segments[0] {
            ToolSegment::Parallel(group) => group.iter().map(|c| c.id.as_str()).collect(),
            ToolSegment::Single(_) => Vec::new(),
        };
        assert_eq!(
            ids,
            vec!["c1", "c2", "c3"],
            "phải là nhóm song song, giữ nguyên thứ tự call"
        );
    }

    #[test]
    fn partition_splits_around_non_safe_calls() {
        let reg = partition_registry();
        let calls = vec![
            call_named("s1", serde_json::json!({})),
            call_named("s2", serde_json::json!({})),
            call_named("d1", serde_json::json!({"dangerous": true})),
            call_named("s3", serde_json::json!({})),
        ];
        let segments = partition_tool_calls(&calls, &reg, true);
        assert_eq!(
            segment_kinds(&segments),
            vec!["parallel", "single", "single"]
        );
    }

    #[test]
    fn partition_respects_config_switch_and_singletons() {
        let reg = partition_registry();
        let calls = vec![
            call_named("c1", serde_json::json!({})),
            call_named("c2", serde_json::json!({})),
        ];
        // Tắt cấu hình: mọi call đều tuần tự.
        assert_eq!(
            segment_kinds(&partition_tool_calls(&calls, &reg, false)),
            vec!["single", "single"]
        );
        // Một call Safe duy nhất: không bọc nhóm song song thừa.
        let one = vec![call_named("c1", serde_json::json!({}))];
        assert_eq!(
            segment_kinds(&partition_tool_calls(&one, &reg, true)),
            vec!["single"]
        );
    }

    #[test]
    fn partition_risk_depends_on_args_like_run_tool_call() {
        let reg = partition_registry();
        // Call đầu mang args nguy hiểm ⇒ không ghép nhóm: risk tính theo args,
        // cùng quy tắc với run_tool_call (mục 7.2).
        let calls = vec![
            call_named("d1", serde_json::json!({"dangerous": true})),
            call_named("s1", serde_json::json!({})),
            call_named("s2", serde_json::json!({})),
        ];
        let segments = partition_tool_calls(&calls, &reg, true);
        assert_eq!(segment_kinds(&segments), vec!["single", "parallel"]);
    }
}
