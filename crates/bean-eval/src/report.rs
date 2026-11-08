//! Metrics: một dòng JSONL mỗi lần chạy task + báo cáo tổng hợp.
//!
//! Đây là dữ liệu để trả lời "cải tiến này có làm tốt hơn không" bằng số:
//! tỉ lệ pass, số bước, token — trước và sau khi đổi prompt/tool/compaction.

use std::io::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::runner::TaskResult;

/// Một dòng trong `metrics.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsRecord {
    /// Id task.
    pub task_id: String,
    /// Đạt mọi khẳng định.
    pub ok: bool,
    /// Số vi phạm.
    pub violation_count: usize,
    /// Lý do kết thúc run.
    pub end_reason: Option<String>,
    /// Số tool call.
    pub tool_calls: usize,
    /// Số confirm tự động trả lời `Allow`.
    pub confirm_count: u32,
    /// Token vào.
    pub input_tokens: u64,
    /// Token ra.
    pub output_tokens: u64,
    /// Số lượt gọi LLM.
    pub llm_calls: usize,
    /// Provider.
    pub provider: String,
    /// Model.
    pub model: String,
    /// Thời gian chạy (ms).
    pub duration_ms: u128,
    /// Thời điểm ghi (RFC3339 UTC).
    pub timestamp: String,
    /// Lỗi run (nếu có).
    pub error: Option<String>,
}

impl MetricsRecord {
    /// Dựng record từ kết quả task.
    #[must_use]
    pub fn from_result(result: &TaskResult) -> Self {
        Self {
            task_id: result.task_id.clone(),
            ok: result.ok,
            violation_count: result.violations.len(),
            end_reason: result.end_reason.clone(),
            tool_calls: result.tool_calls,
            confirm_count: result.confirm_count,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            llm_calls: result.llm_calls,
            provider: result.provider.clone(),
            model: result.model.clone(),
            duration_ms: result.duration_ms,
            timestamp: chrono::Utc::now().to_rfc3339(),
            error: result.error.clone(),
        }
    }

    /// Nối một dòng JSON vào file (tạo file/thư mục cha khi chưa có).
    ///
    /// # Errors
    /// Lỗi khi không tạo/ghi được file.
    pub fn append_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let mut line = serde_json::to_string(self).map_err(std::io::Error::other)?;
        line.push('\n');
        file.write_all(line.as_bytes())
    }

    /// Đọc mọi record từ file JSONL; bỏ qua dòng hỏng (ghi warning qua tracing).
    ///
    /// # Errors
    /// Lỗi khi không đọc được file.
    pub fn read_all(path: &Path) -> anyhow::Result<Vec<Self>> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("không đọc được {}: {error}", path.display()))?;
        let mut records = Vec::new();
        for (index, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Self>(line) {
                Ok(record) => records.push(record),
                Err(error) => tracing::warn!(line = index, %error, "dòng metrics hỏng — bỏ qua"),
            }
        }
        Ok(records)
    }
}

/// Tổng hợp một file metrics — đầu ra của `bean eval report`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskSummary {
    /// Số record đọc được.
    pub total: usize,
    /// Số record đạt.
    pub passed: usize,
    /// Tỉ lệ pass (0.0–1.0).
    pub pass_rate: f64,
    /// Tổng token vào.
    pub input_tokens: u64,
    /// Tổng token ra.
    pub output_tokens: u64,
    /// Trung bình tool call mỗi record (0 khi rỗng).
    pub avg_tool_calls: f64,
    /// Trung bình thời gian mỗi record (ms, 0 khi rỗng).
    pub avg_duration_ms: u64,
    /// Số record lỗi run (khác fail khẳng định).
    pub errored: usize,
}

/// Tổng hợp danh sách record.
#[must_use]
pub fn summarize(records: &[MetricsRecord]) -> TaskSummary {
    let total = records.len();
    let passed = records.iter().filter(|record| record.ok).count();
    let input_tokens: u64 = records.iter().map(|record| record.input_tokens).sum();
    let output_tokens: u64 = records.iter().map(|record| record.output_tokens).sum();
    let tool_calls: u64 = records.iter().map(|record| record.tool_calls as u64).sum();
    let duration: u128 = records.iter().map(|record| record.duration_ms).sum();
    let errored = records
        .iter()
        .filter(|record| record.error.is_some())
        .count();
    TaskSummary {
        total,
        passed,
        pass_rate: if total == 0 {
            0.0
        } else {
            passed as f64 / total as f64
        },
        input_tokens,
        output_tokens,
        avg_tool_calls: if total == 0 {
            0.0
        } else {
            tool_calls as f64 / total as f64
        },
        avg_duration_ms: if total == 0 {
            0
        } else {
            u64::try_from(duration / total as u128).unwrap_or(u64::MAX)
        },
        errored,
    }
}

/// In báo cáo dạng bảng text ra stdout (một dòng mỗi record + dòng tổng).
///
/// # Errors
/// Lỗi khi ghi stdout thất bại.
pub fn print_report(records: &[MetricsRecord]) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(
        out,
        "{:<32} {:>4} {:>6} {:>7} {:>7} {:>8}",
        "task", "ok", "tools", "tok_in", "tok_out", "ms"
    )?;
    for record in records {
        writeln!(
            out,
            "{:<32} {:>4} {:>6} {:>7} {:>7} {:>8}",
            record.task_id,
            if record.ok { "PASS" } else { "FAIL" },
            record.tool_calls,
            record.input_tokens,
            record.output_tokens,
            record.duration_ms,
        )?;
    }
    let summary = summarize(records);
    writeln!(
        out,
        "\ntổng: {}/{} pass ({:.0}%), token {}/{} (in/out), tb tool {:.1}, tb ms {}, lỗi run {}",
        summary.passed,
        summary.total,
        summary.pass_rate * 100.0,
        summary.input_tokens,
        summary.output_tokens,
        summary.avg_tool_calls,
        summary.avg_duration_ms,
        summary.errored,
    )
}
