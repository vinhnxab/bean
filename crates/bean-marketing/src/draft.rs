//! Tool `marketing_draft` — lưu bản nháp nội dung vào workspace (M24).
//!
//! **Tuyệt đối không gọi mạng.** Đây là bảo đảm ở tầng code, không phải lời hứa trong
//! `description`: tool này không còn đường dẫn HTTP nào, nên không thể vô tình đăng bài
//! sớm. Việc lên xuống mạng chỉ nằm ở `marketing_publish`, sau một bước xác nhận.
//!
//! Rủi ro `Confirm` (không phải `Dangerous`) vì nó chỉ ghi file trong workspace đã bị jail —
//! người dùng có thể đọc lại, xoá được; khác với đăng bài ra ngoài.

use std::borrow::Cow;
use std::sync::Arc;

use bean_tools::{Tool, ToolAccess, ToolCtx, ToolError};
use bean_types::config::MARKETING_DRAFT_TAG;
use bean_types::{Risk, ToolSpec};
use schemars::JsonSchema;
use serde::Deserialize;

/// Tham số của `marketing_draft`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarketingDraftParams {
    /// Tên bản nháp, dùng làm tên file. Chỉ chữ/số/gạch dưới/gạch ngang, không có dấu
    /// `/` hay `..` — file jail (`cap-std`) chặn nữa nhưng tool kiểm sớm để báo lỗi rõ.
    pub title: String,
    /// Nội dung bản nháp (văn bản thuần hoặc markdown).
    pub body: String,
    /// Ghi đè nếu bản nháp đã tồn tại. Mặc định `false` để không âm thầm mất nội dung cũ.
    #[serde(default)]
    pub overwrite: bool,
}

/// Trần độ dài nội dung nháp (ký tự) — tránh model đổ một khối text khổng lồ vào file.
const MAX_BODY_CHARS: usize = 20_000;

/// Kiểm tra `title` là tên file an toàn (không đường dẫn).
fn validate_title(title: &str) -> Result<&str, ToolError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(ToolError::InvalidArgs("title rỗng".into()));
    }
    if trimmed.len() > 80 {
        return Err(ToolError::InvalidArgs("title dài quá 80 ký tự".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ToolError::InvalidArgs(
            "title chỉ được chứa chữ, số, `-` và `_` (không dấu `/`, không `..`)".into(),
        ));
    }
    Ok(trimmed)
}

/// Dựng tool `marketing_draft`.
#[must_use]
pub fn marketing_draft() -> Arc<dyn Tool> {
    let spec = ToolSpec::new(
        "marketing_draft",
        "Lưu bản nháp nội dung marketing vào workspace dưới dạng file markdown. \
         Chỉ ghi file, KHÔNG đăng lên bất kỳ nền tảng nào — dùng `marketing_publish` để đăng.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Tên bản nháp, dùng làm tên file. Chỉ chữ/số/gạch dưới/gạch ngang, ví dụ: launch-2026-q3"
                },
                "body": {
                    "type": "string",
                    "description": "Nội dung bản nháp (văn bản thuần hoặc markdown)."
                },
                "overwrite": {
                    "type": "boolean",
                    "description": "Ghi đè nếu bản nháp đã tồn tại. Mặc định false để không mất nội dung cũ."
                }
            },
            "required": ["title", "body"],
            "additionalProperties": false
        }),
    );
    Arc::new(MarketingDraftTool { spec })
}

struct MarketingDraftTool {
    spec: ToolSpec,
}

#[async_trait::async_trait]
impl Tool for MarketingDraftTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        // Ghi file trong workspace đã jail → `Confirm`, không phải `Dangerous`.
        Risk::Confirm
    }

    /// (M24) Chỉ role giữ tag `marketing-draft` mới thấy/cọp tool này.
    fn access(&self) -> ToolAccess<'_> {
        ToolAccess {
            required_tags: Cow::Borrowed(&[MARKETING_DRAFT_TAG]),
            ..ToolAccess::default()
        }
    }

    /// Bản nháp là nội dung **do agent tự soạn** dựa trên đọc web — không phải dữ liệu
    /// ngoài lõi, nên **không** bọc `<untrusted_content>` (khác `web_fetch`, vốn trả về
    /// nội dung của trang).
    fn marks_untrusted(&self) -> bool {
        false
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params: MarketingDraftParams =
            bean_tools::typed::deserialize_params(args, &self.spec.parameters)
                .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;

        let title = validate_title(&params.title)?;
        if params.body.chars().count() > MAX_BODY_CHARS {
            return Err(ToolError::InvalidArgs(format!(
                "nội dung dài quá {MAX_BODY_CHARS} ký tự — rút gọn hoặc chia nhiều bản nháp"
            )));
        }

        // `drafts/` là thư mục riêng để không lẫn với workspace thường của agent.
        let relative = format!("drafts/{title}.md");
        // `WorkspaceFs` không có `exists`; thử đọc là cách kiểm duy nhất mà không đi
        // quanh trait (mọI I/O đều phải qua trait này để giữ path jail).
        let exists = ctx.workspace.read_text(&relative).is_ok();
        if exists && !params.overwrite {
            return Err(ToolError::InvalidArgs(format!(
                "bản nháp `{title}` đã tồn tại — đặt `overwrite = true` nếu thực sự muốn ghi đè"
            )));
        }
        ctx.workspace.write_text(&relative, &params.body)?;
        Ok(format!("Đã lưu bản nháp `{title}` vào {relative}."))
    }
}
