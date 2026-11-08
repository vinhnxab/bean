//! Bộ khẳng định chấm một run eval: run có "hoàn thành tác vụ" hay không.
//!
//! Chỉ ở đây mới định nghĩa "thành công" — runner chỉ ghi nhận sự kiện. Thêm
//! dạng khẳng định mới phải thêm test hành vi tương ứng (mục 20).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::runner::TaskResult;

/// Lý do kết thúc run mà task chấp nhận (so với `EndReason` của agent loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedEnd {
    /// Provider trả lời cuối bình thường.
    Final,
    /// Đạt giới hạn số bước.
    MaxSteps,
    /// Chạm ngân sách token/ngày.
    BudgetExceeded,
    /// Provider kết thúc lượt nhưng không trả chữ nào.
    EmptyResponse,
}

/// Phần `[assert]` của task TOML — mọi trường đều tuỳ chọn; task không có khẳng
/// định nào chỉ đo metrics (vẫn hữu ích để lấy đường cơ sở trước khi tối ưu).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assertions {
    /// Lý do kết thúc run mong muốn.
    #[serde(default)]
    pub end_reason: Option<ExpectedEnd>,
    /// Text cuối phải chứa tất cả chuỗi này.
    #[serde(default)]
    pub final_contains: Vec<String>,
    /// Text cuối không được chứa chuỗi nào trong danh sách.
    #[serde(default)]
    pub final_not_contains: Vec<String>,
    /// Những tool này phải được gọi (thứ tự tuỳ ý).
    #[serde(default)]
    pub tools_called: Vec<String>,
    /// Không tool nào trong danh sách được gọi (ví dụ `run_shell` trong task đọc/ghi).
    #[serde(default)]
    pub tools_forbidden: Vec<String>,
    /// Chuỗi tool call **đúng thứ tự trọn vẹn** (bắt đầu từ tool đầu tiên).
    #[serde(default)]
    pub tool_sequence: Option<Vec<String>>,
    /// Số tool call tối thiểu.
    #[serde(default)]
    pub min_tool_calls: Option<u32>,
    /// Số tool call tối đa.
    #[serde(default)]
    pub max_tool_calls: Option<u32>,
    /// Số lần confirm đúng bằng con số này (0 ⇒ run không được hỏi ai cả).
    #[serde(default)]
    pub confirm_count: Option<u32>,
    /// File trong workspace phải có nội dung **đúng bằng** sau run.
    #[serde(default)]
    pub files_equal: BTreeMap<String, String>,
    /// File trong workspace phải chứa chuỗi.
    #[serde(default)]
    pub files_contain: BTreeMap<String, String>,
    /// `true` ⇒ phiên phải có `sessions.summary` khác rỗng (compaction đã chạy);
    /// `false` ⇒ không được có summary.
    #[serde(default)]
    pub summary_saved: Option<bool>,
    /// Độ dài tối thiểu của summary (ký tự).
    #[serde(default)]
    pub min_summary_chars: Option<usize>,
    /// `true` ⇒ learning loop phải tạo ít nhất một skill nháp sau run.
    #[serde(default)]
    pub draft_created: Option<bool>,
}

impl Assertions {
    /// Chấm [`TaskResult`] — trả danh sách vi phạm (rỗng = đạt).
    ///
    /// Hàm này **thuần**: không I/O, dữ liệu workspace/summary/draft đã được
    /// runner thu thập vào `TaskResult` trước khi gọi.
    #[must_use]
    pub fn evaluate(&self, result: &TaskResult) -> Vec<String> {
        let mut violations = Vec::new();
        if let Some(expected) = self.end_reason
            && result.end_reason.as_deref() != Some(expected.as_str())
        {
            violations.push(format!(
                "end_reason mong `{}` nhưng nhận `{}`",
                expected.as_str(),
                result.end_reason.as_deref().unwrap_or("không rõ")
            ));
        }
        if let Some(error) = &result.error {
            violations.push(format!("run kết thúc bằng lỗi: {error}"));
        }
        let final_text = result.final_text.as_deref().unwrap_or_default();
        for needle in &self.final_contains {
            if !final_text.contains(needle.as_str()) {
                violations.push(format!("text cuối thiếu chuỗi `{needle}`"));
            }
        }
        for needle in &self.final_not_contains {
            if final_text.contains(needle.as_str()) {
                violations.push(format!("text cuối chứa chuỗi cấm `{needle}`"));
            }
        }
        for tool in &self.tools_called {
            if !result.tools_called.iter().any(|called| called == tool) {
                violations.push(format!("tool `{tool}` không được gọi"));
            }
        }
        for tool in &self.tools_forbidden {
            if result.tools_called.iter().any(|called| called == tool) {
                violations.push(format!("tool `{tool}` bị cấm nhưng đã được gọi"));
            }
        }
        if let Some(expected) = &self.tool_sequence
            && result.tools_called != *expected
        {
            violations.push(format!(
                "thứ tự tool mong {expected:?} nhưng nhận {:?}",
                result.tools_called
            ));
        }
        if let Some(min) = self.min_tool_calls
            && result.tool_calls < min as usize
        {
            violations.push(format!("tool call {} < tối thiểu {min}", result.tool_calls));
        }
        if let Some(max) = self.max_tool_calls
            && result.tool_calls > max as usize
        {
            violations.push(format!("tool call {} > tối đa {max}", result.tool_calls));
        }
        if let Some(expected) = self.confirm_count
            && result.confirm_count != expected
        {
            violations.push(format!(
                "số confirm {} ≠ mong đợi {expected}",
                result.confirm_count
            ));
        }
        for (path, expected) in &self.files_equal {
            match result.files.get(path) {
                Some(actual) if actual == expected => {}
                Some(actual) => violations.push(format!(
                    "file `{path}` lệch nội dung:\n--- mong ---\n{expected}\n--- nhận ---\n{actual}"
                )),
                None => violations.push(format!("file `{path}` không tồn tại sau run")),
            }
        }
        for (path, needle) in &self.files_contain {
            match result.files.get(path) {
                Some(actual) if actual.contains(needle.as_str()) => {}
                Some(_) => violations.push(format!("file `{path}` thiếu chuỗi `{needle}`")),
                None => violations.push(format!("file `{path}` không tồn tại sau run")),
            }
        }
        if let Some(expected) = self.summary_saved {
            match (&result.summary, expected) {
                (Some(summary), true) if summary.trim().is_empty() => {
                    violations.push("cần summary khác rỗng nhưng summary rỗng".into());
                }
                (Some(_), true) => {}
                (Some(_), false) => violations.push("summary tồn tại nhưng task cấm".into()),
                (None, true) => {
                    violations.push("không có sessions.summary (compaction chưa chạy)".into())
                }
                (None, false) => {}
            }
        }
        if let Some(min) = self.min_summary_chars {
            let len = result
                .summary
                .as_deref()
                .map_or(0, |summary| summary.chars().count());
            if len < min {
                violations.push(format!("summary {len} ký tự < tối thiểu {min}"));
            }
        }
        if let Some(expected) = self.draft_created {
            let created = result.draft_count > 0;
            if created != expected {
                violations.push(format!(
                    "skill nháp mong `{expected}` nhưng có {} bản",
                    result.draft_count
                ));
            }
        }
        violations
    }
}

impl ExpectedEnd {
    /// Chuỗi so khớp với `TaskResult.end_reason` (snake_case, trùng serde).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::MaxSteps => "max_steps",
            Self::BudgetExceeded => "budget_exceeded",
            Self::EmptyResponse => "empty_response",
        }
    }
}
