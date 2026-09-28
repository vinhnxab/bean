//! Tool `qa_test` — chạy test suite đã khai báo (M27).
//!
//! Bốn bất biến của milestone này đã nêu ở [crate] docs. Ở đây chỉ ghi phần thực thi:
//!
//! * Không có tham số `command`/`argv`/`workdir` — cùng lý do với `security_scan` của M23 (D14.5).
//! * Tra catalog **trước** khi spawn; catalog rỗng hoặc tên lạ ⇒ từ chối, bật cờ
//!   `untrusted_seen` như M23 vì lượt này đã chạm vào dữ liệu ngoài lõi.
//! * `Confirm` (không phải `Dangerous`) ⇒ `Policy::decide` trả `allow_in_session: true`.
//!   Xem `docs/decisions.md` D17.3 về lý do và điều kiện ràng buộc quyết định đó.
//! * `marks_untrusted = true` ⇒ cả `QaReport` lẫn raw log đều bọc `<untrusted_content>`.

use std::sync::Arc;

use beanagent_security::{Sandbox, SandboxError};
use beanagent_tools::{Tool, ToolCtx, ToolError, TypedTool};
use beanagent_types::Risk;
use beanagent_types::config::TEST_RUN_TAG;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::report::{QaReport, parse_output};
use crate::suite::{SuiteCatalog, build_argv, sanitize_filter};

/// Tham số của `qa_test`.
///
/// **Cố ý KHÔNG có tham số `command`/`argv`/`workdir`.** Model chỉ mô tả *ý định* ("chạy
/// suite core-unit, lọc test parse_path"); mọi argv thực sự do [`build_argv`] sinh trong
/// code từ `[[qa.suites]]`. Đây là ranh giới giữa "agent được yêu cầu chạy test" và "agent
/// được chạy lệnh tuỳ ý" (D14.5).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QaTestParams {
    /// Tên suite đã khai báo trong `[[qa.suites]]`. Chạy suite không có trong danh sách sẽ
    /// bị từ chối ở tầng code.
    pub suite_name: String,
    /// Lọc theo tên test (tùy chọn), ví dụ `test_parse_path` hoặc `tools::`. Bỏ trống để
    /// chạy toàn bộ suite.
    #[serde(default)]
    pub filter: Option<String>,
}

/// Dựng tool `qa_test` từ catalog + sandbox.
#[must_use]
pub fn qa_test(catalog: SuiteCatalog, sandbox: Arc<Sandbox>) -> Arc<dyn Tool> {
    let empty = catalog.is_empty();
    let names = catalog.names().join(", ");
    let mut spec = beanagent_tools::typed_spec::<QaTestParams>("qa_test");
    // Mô tả liệt kê đúng tên suite đang khai báo: model không có cách nào đoán mò, và
    // danh sách rỗng thì phải nói thẳng là chưa có gì để chạy (fail-closed, D14.1).
    spec.description = if empty {
        "Chạy một test suite của dự án trong container với workspace ở chế độ ĐỌC-ONLY. \
         HIỆN CHƯA KHAI BÁO SUITE NÀO ([[qa.suites]] rỗng) — mọi lần gọi sẽ bị từ chối. \
         Bạn KHÔNG thể tự tạo test hay ghi file test: đó là việc của vai trò developer."
            .to_string()
    } else {
        format!(
            "Chạy một test suite đã khai báo của dự án (workspace ĐỌC-ONLY, không ghi được \
             file nào, kể cả file test). Suite hiện có: {names}. Chỉ chạy test có sẵn — \
             nếu cần viết test mới thì đó là việc của vai trò developer, không phải của bạn."
        )
    };
    Arc::new(
        TypedTool::new(
            "qa_test",
            Risk::Confirm,
            move |ctx: &ToolCtx, p: QaTestParams| {
                let catalog = catalog.clone();
                let sandbox = Arc::clone(&sandbox);
                // `async move` không được mượn `ctx` (xem docs `typed.rs`): lấy trước
                // những gì handler cần rồi mới vào block.
                let cancel = ctx.cancel.clone();
                let untrusted_seen = ctx.untrusted_seen.clone();
                async move {
                    // (1) TRA CATALOG TRƯỚC TIÊN — tầng code, không dựa vào model (D14.1).
                    let Some(suite) = catalog.get(&p.suite_name) else {
                        let (reason, hint) = if catalog.is_empty() {
                            (
                                "chưa khai báo mục `[[qa.suites]]` nào".to_string(),
                                String::new(),
                            )
                        } else {
                            (
                                format!("`{}` không có trong [[qa.suites]]", p.suite_name.trim()),
                                format!(" Suite hiện có: {}.", catalog.names().join(", ")),
                            )
                        };
                        // Bật cờ untrusted: lượt này đã chạm vào nguồn ngoài lõi (mục 15.4).
                        untrusted_seen.store(true, std::sync::atomic::Ordering::SeqCst);
                        return Ok(wrap(&format!(
                            "TỪ CHỐI: {reason}.{hint} Chỉ được chạy suite đã khai báo trong \
                             BeanAgent.toml; không thể tự ý chạy lệnh khác, và cũng không thể \
                             tự viết test mới (việc đó thuộc vai trò developer)."
                        )));
                    };

                    // (2) Làm sạch `filter` ở ranh giới rồi **tự dựng** argv (D14.5).
                    let filter = sanitize_filter(p.filter.as_deref().unwrap_or_default())
                        .map_err(|reason| ToolError::InvalidArgs(format!("filter: {reason}")))?;
                    let argv = build_argv(suite.runner, &suite.fixed_args, filter.as_deref());

                    // (3) Chạy trong container với workspace mount READ-ONLY.
                    let timeout = std::time::Duration::from_secs(sandbox.timeout_seconds());
                    let outcome = sandbox
                        .run_argv_readonly(&argv, &suite.workdir, timeout, &cancel)
                        .await
                        .map_err(sandbox_error)?;

                    let mut output = outcome.stdout;
                    if !outcome.stderr.is_empty() {
                        output.push_str("\n--- stderr ---\n");
                        output.push_str(&outcome.stderr);
                    }
                    let report =
                        parse_output(suite.runner, &suite.name, outcome.exit_code, &output);
                    Ok(wrap(&format!(
                        "{}\n--- raw log ---\n{output}",
                        render_json(&report)
                    )))
                }
            },
        )
        .description(spec.description)
        // Mục 15.4 / D17.4: tên test, fixture, assertion message đều do code dự án kiểm soát
        // ⇒ bọc untrusted cho **cả** report lẫn raw log, không có ngoại lệ "đã chuẩn hoá".
        .untrusted()
        // RBAC (M21.4): chỉ role giữ tag `test-run` mới thấy/cọp tool này. Role `qa` có tag
        // này; `developer`/`security-scan` không có nên **không** chạy được test thay QA.
        .requires_tags([TEST_RUN_TAG]),
    )
}

/// Serialize báo cáo; struct phẳng này không thất bại trong thực tế nhưng vẫn có fallback.
fn render_json(report: &QaReport) -> String {
    serde_json::to_string(report).unwrap_or_else(|_| r#"{"status":"error"}"#.to_string())
}

/// Bọc untrusted với trần cứng (thẻ đóng luôn còn, mục 15.4).
fn wrap(content: &str) -> String {
    beanagent_tools::wrap_bounded(content, beanagent_tools::MAX_WRAPPED_OUTPUT_CHARS)
}

/// Chuyển lỗi sandbox thành `ToolError`.
fn sandbox_error(error: SandboxError) -> ToolError {
    let message = match error {
        SandboxError::Timeout(seconds) => {
            format!("test hết thời gian cho phép ({seconds}s) — đã bị kill")
        }
        SandboxError::Cancelled => "bạn đã huỷ lần chạy test".to_string(),
        SandboxError::Launch(detail) => format!("không chạy được test: {detail}"),
    };
    ToolError::Io(beanagent_tools::wrap_untrusted(&message))
}
