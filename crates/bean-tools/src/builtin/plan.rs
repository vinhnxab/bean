//! Tool `update_plan` (D25) — lập kế hoạch có cấu trúc mà model cập nhật được.
//!
//! # Vì sao tồn tại
//!
//! Việc dài nhiều bước dễ lạc hướng: model quên các bước còn lại hoặc làm trùng
//! bước đã xong. Kế hoạch không phải dữ liệu workspace mà là **trạng thái sống của
//! lượt**: agent loop phát nó thành `RunEvent::Plan` (kênh web hiển thị thành thẻ
//! kế hoạch), lịch sử vẫn giữ cặp tool_use/tool_result như mọi tool khác.
//!
//! # Bất biến
//!
//! * **`Safe`** — không chạm file, mạng hay DB; chỉ phát sự kiện hiển thị.
//! * Snapshot **ghi đè**: mỗi lần gọi gửi **toàn bộ** danh sách bước; không có API
//!   "thêm/xoá một bước" để trạng thái luôn khớp một nguồn duy nhất.
//! * Giới hạn: 1..=20 bước, tiêu đề 1..=200 ký tự — vừa khớt preview cắt ~2.000
//!   ký tự, vừa không cho plan phình thành hệ thống quản lý công việc.

use std::sync::Arc;

use bean_types::{PlanStep, PlanStepStatus, Risk};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;
use crate::typed::TypedTool;

/// Tên tool kế hoạch — agent loop dùng hằng này để nhận diện và phát `on_plan`.
pub const PLAN_TOOL: &str = "update_plan";

/// Trần số bước một kế hoạch được phép có.
const MAX_STEPS: usize = 20;

/// Trần ký tự của tiêu đề một bước (theo ký tự Unicode, không phải byte).
const MAX_TITLE_CHARS: usize = 200;

/// Tham số `update_plan`. Kế hoạch gửi nguyên danh sách mỗi lần.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdatePlanParams {
    /// Toàn bộ các bước của kế hoạch theo thứ tự thực hiện. Gửi lại toàn bộ mỗi lần
    /// muốn cập nhật (thêm bước, đánh dấu bước đang làm/xong).
    steps: Vec<PlanStep>,
}

/// Parse tham số `update_plan` thành danh sách [`PlanStep`] cho agent loop phát
/// `on_plan` **trước khi** tool chạy. `None` khi args sai schema **hoặc** vi phạm
/// ràng buộc (danh sách rỗng, quá trần, tiêu đề rỗng/quá dài) — khi đó tool sẽ trả
/// lỗi rõ ràng cho model tự sửa, không phát kế hoạch sai ra UI.
#[must_use]
pub fn parse_plan_steps(args: &serde_json::Value) -> Option<Vec<PlanStep>> {
    let steps = serde_json::from_value::<UpdatePlanParams>(args.clone())
        .ok()?
        .steps;
    validate_steps(&steps).ok()?;
    Some(steps)
}

/// Ràng buộc dùng chung cho một kế hoạch hợp lệ — agent loop (qua
/// `parse_plan_steps`) và tool (thông báo lỗi chi tiết cho model) cùng dùng một
/// nguồn chân lý để không bao giờ phát ra UI kế hoạch mà tool sẽ từ chối.
fn validate_steps(steps: &[PlanStep]) -> Result<(), ToolError> {
    if steps.is_empty() {
        return Err(ToolError::InvalidArgs(
            "`steps` phải có ít nhất 1 bước. Nếu nhiệm vụ chỉ một bước thì không cần kế hoạch."
                .into(),
        ));
    }
    if steps.len() > MAX_STEPS {
        return Err(ToolError::InvalidArgs(format!(
            "`steps` tối đa {MAX_STEPS} bước — gộp các việc nhỏ lại."
        )));
    }
    for (index, step) in steps.iter().enumerate() {
        let title_chars = step.title.chars().count();
        if step.title.trim().is_empty() {
            return Err(ToolError::InvalidArgs(format!(
                "Bước {} thiếu tiêu đề (`title` không được rỗng).",
                index + 1
            )));
        }
        if title_chars > MAX_TITLE_CHARS {
            return Err(ToolError::InvalidArgs(format!(
                "Tiêu đề bước {} dài {title_chars} ký tự — tối đa {MAX_TITLE_CHARS}. Hãy rút gọn.",
                index + 1
            )));
        }
    }
    Ok(())
}

/// Cập nhật kế hoạch các bước của nhiệm vụ đang làm.
///
/// Dùng ngay từ đầu nhiệm vụ nhiều bước để liệt kê các bước, rồi cập nhật sau mỗi
/// bước (đánh dấu `done`, chuyển bước kế tiếp sang `in_progress`). Luôn gửi **toàn
/// bộ** danh sách — lần gọi sau thay thế lần trước.
#[must_use]
pub fn tool() -> Arc<dyn Tool> {
    Arc::new(TypedTool::new(
        PLAN_TOOL,
        Risk::Safe,
        |_: &ToolCtx, params: UpdatePlanParams| {
            let steps = params.steps;
            async move { validate_and_summarize(&steps) }
        },
    ))
}

fn validate_and_summarize(steps: &[PlanStep]) -> Result<String, ToolError> {
    validate_steps(steps)?;
    let mut done = 0usize;
    let mut in_progress = 0usize;
    for step in steps {
        match step.status {
            PlanStepStatus::Done => done += 1,
            PlanStepStatus::InProgress => in_progress += 1,
            PlanStepStatus::Pending => {}
        }
    }
    Ok(format!(
        "Đã cập nhật kế hoạch: {} bước ({} hoàn thành, {} đang làm, {} chưa bắt đầu).",
        steps.len(),
        done,
        in_progress,
        steps.len() - done - in_progress
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;

    fn step(title: &str, status: &str) -> serde_json::Value {
        json!({ "title": title, "status": status })
    }

    #[test]
    fn parse_steps_hop_le_tra_dung_thu_tu() {
        let args =
            json!({ "steps": [step("a", "done"), step("b", "in_progress"), step("c", "pending")] });
        let steps = parse_plan_steps(&args).unwrap_or_default();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].status, PlanStepStatus::Done);
        assert_eq!(steps[1].status, PlanStepStatus::InProgress);
        assert_eq!(steps[2].status, PlanStepStatus::Pending);
        assert_eq!(steps[2].title, "c");
    }

    #[test]
    fn parse_steps_sai_schema_hoac_vi_pham_rang_buoc_tra_none() {
        assert!(parse_plan_steps(&json!({ "steps": "không phải mảng" })).is_none());
        // `deny_unknown_fields`: trường thừa ở từng bước phải bị từ chối, không lọt.
        assert!(
            parse_plan_steps(
                &json!({ "steps": [json!({"title": "a", "status": "pending", "x": 1})] })
            )
            .is_none()
        );
        assert!(parse_plan_steps(&json!({})).is_none());
        // Ràng buộc nghiệp vụ: agent loop không được phát kế hoạch tool sẽ từ chối.
        assert!(parse_plan_steps(&json!({ "steps": [] })).is_none());
        let too_many: Vec<serde_json::Value> = (0..(MAX_STEPS + 1))
            .map(|i| step(&format!("bước {i}"), "pending"))
            .collect();
        assert!(parse_plan_steps(&json!({ "steps": too_many })).is_none());
        assert!(parse_plan_steps(&json!({ "steps": [step("   ", "pending")] })).is_none());
        let long_title: String = "ạ".repeat(MAX_TITLE_CHARS + 1);
        assert!(parse_plan_steps(&json!({ "steps": [step(&long_title, "pending")] })).is_none());
    }

    #[tokio::test]
    async fn tool_tu_choi_danh_sach_rong_va_vuot_tran() {
        let tool = tool();
        let ctx = test_ctx();
        let empty = tool.call(&ctx, json!({ "steps": [] })).await;
        assert!(empty.is_err());
        let too_many: Vec<serde_json::Value> = (0..21)
            .map(|i| step(&format!("bước {i}"), "pending"))
            .collect();
        let over = tool.call(&ctx, json!({ "steps": too_many })).await;
        assert!(over.is_err());
    }

    #[tokio::test]
    async fn tool_tu_choi_tieu_de_rong_va_qua_dai() {
        let tool = tool();
        let ctx = test_ctx();
        let empty_title = tool
            .call(&ctx, json!({ "steps": [step("   ", "pending")] }))
            .await;
        assert!(empty_title.is_err());
        let long_title: String = "ạ".repeat(MAX_TITLE_CHARS + 1);
        let too_long = tool
            .call(&ctx, json!({ "steps": [step(&long_title, "pending")] }))
            .await;
        assert!(too_long.is_err());
        // Đúng trần (200 ký tự có dấu) thì chấp nhận — cắt theo ký tự, không byte.
        let exact: String = "ạ".repeat(MAX_TITLE_CHARS);
        let ok = tool
            .call(&ctx, json!({ "steps": [step(&exact, "pending")] }))
            .await;
        assert!(ok.is_ok());
    }

    #[tokio::test]
    async fn tom_tat_dem_dung_trang_thai() {
        let args = json!({
            "steps": [step("xong", "done"), step("đang", "in_progress"), step("chưa", "pending")]
        });
        let tool = tool();
        let ctx = test_ctx();
        let output = tool.call(&ctx, args).await.expect("plan hợp lệ phải Ok");
        assert!(output.contains("3 bước"), "text: {output}");
        assert!(output.contains("1 hoàn thành"), "text: {output}");
        assert!(output.contains("1 đang làm"), "text: {output}");
        assert!(output.contains("1 chưa bắt đầu"), "text: {output}");
    }

    fn test_ctx() -> ToolCtx {
        // Kế hoạch không đụng workspace; các trường còn lại chỉ để điền đủ struct.
        ToolCtx::for_project(
            Arc::new(StubWorkspace),
            bean_types::SessionId::new(1),
            tokio_util::sync::CancellationToken::new(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
    }

    #[derive(Debug)]
    struct StubWorkspace;

    impl crate::workspace::WorkspaceFs for StubWorkspace {
        fn root(&self) -> &std::path::Path {
            std::path::Path::new(".")
        }

        fn resolve(&self, rel: &str) -> Result<std::path::PathBuf, ToolError> {
            Ok(std::path::PathBuf::from(rel))
        }

        fn read_range(&self, rel: &str, offset: u64, limit: u64) -> Result<Vec<u8>, ToolError> {
            let text = self.read_text(rel)?;
            let start = usize::try_from(offset).unwrap_or(0).min(text.len());
            let end = (start + usize::try_from(limit).unwrap_or(0)).min(text.len());
            Ok(text.as_bytes()[start..end].to_vec())
        }

        fn read_text(&self, _rel: &str) -> Result<String, ToolError> {
            Err(ToolError::Internal("stub".into()))
        }

        fn write_text(&self, _rel: &str, _content: &str) -> Result<(), ToolError> {
            Err(ToolError::Internal("stub".into()))
        }

        fn edit_unique(&self, _rel: &str, _old: &str, _new: &str) -> Result<(), ToolError> {
            Err(ToolError::Internal("stub".into()))
        }

        fn list_dir(&self, _rel: &str) -> Result<Vec<crate::DirEntryInfo>, ToolError> {
            Ok(Vec::new())
        }

        fn glob(&self, _pattern: &str, _limit: usize) -> Result<Vec<String>, ToolError> {
            Ok(Vec::new())
        }

        fn grep(
            &self,
            _pattern: &str,
            _rel: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<crate::GrepMatch>, ToolError> {
            Ok(Vec::new())
        }
    }
}
