//! Mô hình task eval: khai báo TOML một tác vụ, setup workspace và kỳ vọng.
//!
//! Một task eval **không** đụng dữ liệu của người dùng: runner tạo thư mục tạm
//! riêng cho workspace/SQLite/skills của từng lượt chạy (xem [`crate::runner`]).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::assertion::Assertions;

/// Task eval đọc từ một file TOML.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvalTask {
    /// Định danh task, dùng trong metrics, trajectory và `--only`.
    pub id: String,
    /// Mô tả người đọc — task này đo gì.
    pub description: String,
    /// Tin nhắn người dùng gửi cho agent trong lượt eval.
    pub prompt: String,
    /// Ghi đè `agent.max_steps` (mặc định của runner: 12 — eval không cần 25 bước).
    #[serde(default)]
    pub max_steps: Option<u32>,
    /// Ghi đè `agent.context_budget_tokens` (tối thiểu 1000 theo validate của Config).
    #[serde(default)]
    pub context_budget_tokens: Option<u32>,
    /// Provider: kịch bản giả hoặc cấu hình provider thật.
    #[serde(default)]
    pub llm: TaskLlm,
    /// Dựng workspace trước khi run: ghi file, dựng lịch sử dài (compaction).
    #[serde(default)]
    pub setup: TaskSetup,
    /// Bộ khẳng định chấm kết quả.
    #[serde(default)]
    pub assert: Assertions,
    /// Trần thời gian chờ một run (giây). Task fake chạy trong vài giây; task
    /// provider thật cần trần lớn hơn.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

fn default_timeout_seconds() -> u64 {
    120
}

/// Phần `[llm]` của task.
///
/// Có `script` ⇒ dùng [`bean_llm::FakeProvider`] — task **xác định**, chạy được
/// trong CI. Không có `script` ⇒ dùng provider thật dựng từ `config.llm` (có thể
/// ghi đè từng trường bên dưới); key **chỉ** đọc từ biến môi trường.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLlm {
    /// Kịch bản JSON của FakeProvider (định dạng `{"responses": [...]}`).
    #[serde(default)]
    pub script: Option<String>,
    /// `anthropic` | `openai_compat` — ghi đè `[llm].provider`.
    #[serde(default)]
    pub provider: Option<bean_types::config::LlmProviderKind>,
    /// Ghi đè `[llm].model`.
    #[serde(default)]
    pub model: Option<String>,
    /// Ghi đè `[llm].base_url` (Ollama/vLLM/OpenRouter).
    #[serde(default)]
    pub base_url: Option<String>,
    /// Ghi đè `[llm].api_key_env` — tên **biến môi trường**, không bao giờ là giá trị key.
    #[serde(default)]
    pub api_key_env: Option<String>,
}

impl TaskLlm {
    /// `true` khi task chạy với FakeProvider (xác định, không tốn tiền).
    #[must_use]
    pub fn is_fake(&self) -> bool {
        self.script.is_some()
    }
}

/// Phần `[setup]`: dựng nội dung workspace trước khi run.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSetup {
    /// File ghi vào workspace (đường dẫn tương đối trong workspace, không thoát ra).
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// Dựng lịch sử dài **trước** lượt eval — dùng để kích hoạt compaction thật.
    #[serde(default)]
    pub prefill: Option<PrefillSpec>,
}

/// `[setup.prefill]` — ghim `messages` tin nhắn user/assistant xen kẽ vào phiên
/// mà Router sẽ chọn, để tổng token vượt ngưỡng 70% ngân sách và kích hoạt
/// compaction ngay đầu run (agents.md mục 8.3).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrefillSpec {
    /// Tổng số message xen kẽ user/assistant (phải chẵn để kết bằng assistant).
    pub messages: u32,
    /// Nội dung mỗi tin user.
    pub user_text: String,
    /// Nội dung mỗi tin assistant.
    pub assistant_text: String,
}

/// Nạp mọi `*.toml` trong thư mục task, sắp theo `id` để thứ tự chạy ổn định.
///
/// # Errors
/// Lỗi khi thư mục không đọc được hoặc một file TOML sai schema — báo đúng tên
/// file để người viết task sửa nhanh.
pub fn load_tasks_dir(dir: &Path) -> anyhow::Result<Vec<EvalTask>> {
    let mut tasks = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|error| {
        anyhow::anyhow!("không đọc được thư mục task {}: {error}", dir.display())
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| anyhow::anyhow!("đọc danh sách task lỗi: {e}"))?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "toml") {
            continue;
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| anyhow::anyhow!("không đọc được task {}: {error}", path.display()))?;
        let task: EvalTask = toml::from_str(&raw)
            .map_err(|error| anyhow::anyhow!("task {} sai schema: {error}", path.display()))?;
        if task.id.is_empty() {
            anyhow::bail!("task {} thiếu `id`", path.display());
        }
        tasks.push(task);
    }
    if tasks.is_empty() {
        anyhow::bail!("không có task *.toml nào trong {}", dir.display());
    }
    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(tasks)
}
