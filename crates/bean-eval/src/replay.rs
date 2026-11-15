//! Replay quỹ đạo một run eval: đọc lại `trajectory/<task_id>.json` và render
//! dạng text dễ đọc để trả lời câu hỏi *vì sao model chọn sai tool*.
//!
//! Băng ghi do runner tạo (`TrajectoryFile`); module này chỉ nạp lại và hiển
//! thị — không tự thu sự kiện, tránh hai đường ghi lệch schema.

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::runner::TrajectoryEvent;

/// Băng ghi đầy đủ của một run, ghi `trajectory/<task_id>.json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct TrajectoryFile {
    /// Task tạo ra run này.
    pub task_id: String,
    /// Tên provider (`fake`, `openai_compat`, ...).
    pub provider: String,
    /// Model cấu hình.
    pub model: String,
    /// Chuỗi sự kiện theo thứ tự thời gian.
    pub events: Vec<TrajectoryEvent>,
    /// Text cuối nếu run kết thúc bình thường.
    pub final_text: Option<String>,
}

/// Nạp một file trajectory do runner ghi.
///
/// # Errors
/// Lỗi khi không đọc/parse được file — báo đúng đường dẫn để người chạy eval
/// biết băng ghi nào hỏng thay vì im lặng render rỗng.
pub fn load_trajectory(path: &Path) -> anyhow::Result<TrajectoryFile> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("đọc trajectory {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse trajectory {}", path.display()))
}

/// Render quỹ đạo thành text: mỗi sự kiện một dòng, đánh số theo thứ tự thời gian.
///
/// Dòng `tool_end` kèm `ok/lỗi` — cặp start/end cùng tool cho biết tool nào
/// thất bại và model có tự sửa sau đó không; đây là phần dùng nhiều nhất khi
/// debug "vì sao chọn sai tool".
#[must_use]
pub fn render_trajectory(trajectory: &TrajectoryFile) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# trajectory {} · provider {} · model {}\n",
        trajectory.task_id, trajectory.provider, trajectory.model
    ));
    for (index, event) in trajectory.events.iter().enumerate() {
        let line = match event {
            TrajectoryEvent::ToolStart { tool, args_preview } => format!(
                "{:>3} tool_start {} {}",
                index + 1,
                tool,
                args_preview.replace('\n', " ")
            ),
            TrajectoryEvent::ToolEnd { tool, ok: true } => {
                format!("{:>3} tool_end   {} ok", index + 1, tool)
            }
            TrajectoryEvent::ToolEnd { tool, ok: false } => {
                format!("{:>3} tool_end   {} LỖI", index + 1, tool)
            }
            TrajectoryEvent::Confirm { risk, prompt } => format!(
                "{:>3} confirm    [{}] {}",
                index + 1,
                risk,
                prompt.replace('\n', " ")
            ),
            TrajectoryEvent::Text { text } => {
                format!("{:>3} text       {}", index + 1, text.replace('\n', " "))
            }
            TrajectoryEvent::Final { text } => {
                format!("{:>3} final      {}", index + 1, text.replace('\n', " "))
            }
            TrajectoryEvent::Error { code, message } => {
                format!(
                    "{:>3} error      [{}] {}",
                    index + 1,
                    code,
                    message.replace('\n', " ")
                )
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn sample() -> TrajectoryFile {
        TrajectoryFile {
            task_id: "fake_demo".into(),
            provider: "fake".into(),
            model: "fake-script".into(),
            events: vec![
                TrajectoryEvent::ToolStart {
                    tool: "read_file".into(),
                    args_preview: r#"path="reports/sai.md""#.into(),
                },
                TrajectoryEvent::ToolEnd {
                    tool: "read_file".into(),
                    ok: false,
                },
                TrajectoryEvent::ToolStart {
                    tool: "glob".into(),
                    args_preview: r#"pattern="reports/*.md""#.into(),
                },
                TrajectoryEvent::ToolEnd {
                    tool: "glob".into(),
                    ok: true,
                },
                TrajectoryEvent::Confirm {
                    risk: "Confirm".into(),
                    prompt: "write_file path=out.md".into(),
                },
                TrajectoryEvent::Final {
                    text: "Xong rồi.".into(),
                },
            ],
            final_text: Some("Xong rồi.".into()),
        }
    }

    #[test]
    fn render_numbered_lines_with_tool_outcome() {
        let rendered = render_trajectory(&sample());
        assert!(rendered.contains("# trajectory fake_demo · provider fake"));
        // Đánh số thứ tự từ 1.
        assert!(rendered.contains("  1 tool_start read_file"));
        // Tool lỗi phải thấy rõ — đúng mục đích debug của replay.
        assert!(rendered.contains("tool_end   read_file LỖI"));
        assert!(rendered.contains("tool_end   glob ok"));
        assert!(rendered.contains("confirm    [Confirm]"));
        assert!(rendered.contains("final      Xong rồi."));
    }

    #[test]
    fn load_roundtrip_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake_demo.json");
        std::fs::write(&path, serde_json::to_string_pretty(&sample()).unwrap()).unwrap();
        let loaded = load_trajectory(&path).unwrap();
        assert_eq!(loaded.task_id, "fake_demo");
        assert_eq!(loaded.events.len(), 6);
        assert!(render_trajectory(&loaded).contains("tool_end   glob ok"));
    }

    #[test]
    fn load_reports_missing_file_with_path() {
        let error = load_trajectory(Path::new("/khong-ton-tai/x.json")).unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains("/khong-ton-tai/x.json"),
            "lỗi phải nêu đúng đường dẫn: {text}"
        );
    }
}
