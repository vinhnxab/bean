//! Bảng thiết lập thống nhất — `SettingsSchema` (M29 mục 4.2) và [`SettingsService`].
//!
//! Một khoá cấu hình có đúng một dòng trong [`SETTINGS`]: đường dẫn, kiểu, tầng
//! (A = UI + CLI; B = chỉ CLI tương tác), áp dụng (`hot` / `restart`) và nơi lưu
//! (`bean.toml` / `llm-state.json` / `voice-state.json`). Khoá chưa có trong bảng thì
//! `config set` **từ chối** — không ghi tuỳ ý vào TOML (M29 mục 4.2).
//!
//! [`SettingsService`] áp thay đổi:
//! * `bean.toml` — sửa bằng `toml_edit` giữ nguyên comment/định dạng người viết, rồi
//!   **validate file sau khi sửa** (`Config::validate`) trước khi ghi atomic; cấu hình
//!   sai bị từ chối ngay, không để lần khởi động sau mới chết.
//! * `llm-state.json` / `voice-state.json` — trạng thái runtime, ghi atomic `0600`
//!   có sẵn trong `bean-types`.
//! * Mọi lần ghi (set/unset) để lại một dòng audit `{khoá, tầng, nguồn cũ→mới, actor}`
//!   — **không bao giờ** chứa giá trị.
//!
//! Secret không nằm trong bảng này: secret dùng slot riêng (`bean secret set`) vì giá
//! trị không bao giờ đi qua `config set` hay hiển thị — `*_env` trong bảng chỉ là tên
//! biến môi trường. Cấu trúc TOML phức tạp (`[[roles]]`, `[[infra_scope]]`,
//! `[[mcp_clients]]`, `agent.user_roles`…) cũng không có dòng trong bảng: `config set`
//! từ chối và hướng người dùng sửa trực tiếp `bean.toml` (tầng B, cần restart).

use std::path::{Path, PathBuf};

use bean_security::{AuditLog, entry_now};
use bean_types::config::{
    Config, DEFAULT_CONFIG_FILE, SETTINGS_STATE_FILE, SettingsState, SpeechProviderKind,
    apply_persisted_settings_state,
};
use serde_json::json;

use crate::SettingsError;

/// Kiểu dữ liệu của một khoá thiết lập.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    /// Đúng/sai.
    Bool,
    /// Số nguyên không dấu 32 bit.
    U32,
    /// Số nguyên không dấu 64 bit (bị chặn ở `i64::MAX` để ghi được TOML).
    U64,
    /// Số thực.
    F64,
    /// Chuỗi.
    Str,
    /// Chuỗi tuỳ chọn — nhập rỗng nghĩa là xoá (bỏ ghi đè).
    OptStr,
    /// Danh sách chuỗi: JSON `["a","b"]` hoặc phân tách phẩy `a, b`.
    StrList,
    /// Danh sách số nguyên: JSON hoặc phân tách phẩy.
    I64List,
}

impl SettingKind {
    /// Nhãn hiển thị cho CLI/UI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Bool => "đúng/sai",
            Self::U32 | Self::U64 => "số nguyên",
            Self::F64 => "số thực",
            Self::Str => "chuỗi",
            Self::OptStr => "chuỗi (bỏ trống = xoá)",
            Self::StrList => "danh sách chuỗi",
            Self::I64List => "danh sách số",
        }
    }
}

/// Tầng khoá cấu hình theo D19.4 — web session bị chiếm không được trở thành đường
/// sửa các khoá bảo mật.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingTier {
    /// Sửa được từ UI và CLI.
    A,
    /// UI chỉ xem; sửa chỉ qua CLI tương tác, cần TTY và xác nhận `y/N`.
    B,
}

impl SettingTier {
    /// Nhãn đầy đủ.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::A => "A (UI + CLI)",
            Self::B => "B (chỉ CLI)",
        }
    }

    /// Ký hiệu ngắn cho bảng `config list`.
    #[must_use]
    pub fn short(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }

    /// Tầng B bắt buộc CLI tương tác — UI không được phép sửa (chỉ xem + gợi ý lệnh).
    #[must_use]
    pub fn cli_only(self) -> bool {
        matches!(self, Self::B)
    }
}

/// Khoá có hiệu lực ngay ở tiến trình sau hay phải restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingApply {
    /// Áp dụng cho run/tiến trình kế tiếp.
    Hot,
    /// Ghi được nhưng phải `beanctl restart` mới có hiệu lực.
    Restart,
}

impl SettingApply {
    /// Nhãn hiển thị.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Hot => "hot",
            Self::Restart => "restart",
        }
    }
}

/// Nơi lưu của một khoá (M29 mục 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingStore {
    /// `bean.toml` — cấu hình tĩnh, không bao giờ chứa key.
    BeanToml,
    /// `data.dir/settings-state.json` — lựa chọn runtime (llm profile/model + voice).
    SettingsState,
}

impl SettingStore {
    /// Nhãn hiển thị.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::BeanToml => "bean.toml",
            Self::SettingsState => "settings-state.json",
        }
    }
}

/// Nguồn của giá trị đang hiển thị — cách giải bẫy "file thắng toml" (AGENTS 22.24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingSource {
    /// Giá trị serde mặc định (khoá không có trong file).
    Default,
    /// Trong `bean.toml` (seed, chưa bị state ghi đè).
    BeanToml,
    /// Trong `data.dir/settings-state.json`.
    SettingsState,
}

impl SettingSource {
    /// Nhãn tiếng Việt hiển thị CLI/UI.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "mặc định",
            Self::BeanToml => "bean.toml",
            Self::SettingsState => "settings-state.json",
        }
    }
}

/// Giá trị đã parse của một khoá — kiểu khớp [`SettingKind`] của khoá.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    U32(u32),
    U64(u64),
    F64(f64),
    Str(String),
    OptStr(Option<String>),
    StrList(Vec<String>),
    I64List(Vec<i64>),
}

impl SettingValue {
    /// Chuỗi hiển thị cho CLI/UI. Bảng thiết lập không chứa secret nên việc hiển thị
    /// giá trị là an toàn.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Bool(value) => value.to_string(),
            Self::U32(value) => value.to_string(),
            Self::U64(value) => value.to_string(),
            Self::F64(value) => value.to_string(),
            Self::Str(value) | Self::OptStr(Some(value)) => value.clone(),
            Self::OptStr(None) => String::new(),
            Self::StrList(items) => format!("[{}]", items.join(", ")),
            Self::I64List(items) => {
                let rendered: Vec<String> = items.iter().map(i64::to_string).collect();
                format!("[{}]", rendered.join(", "))
            }
        }
    }
}

/// Một dòng của bảng thiết lập — mô tả đầy đủ một khoá cấu hình (M29 mục 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingEntry {
    /// Đường dẫn khoá, ví dụ `llm.model` hoặc `security.sandbox.image`.
    pub path: &'static str,
    /// Kiểu giá trị.
    pub kind: SettingKind,
    /// Tầng A/B theo D19.4.
    pub tier: SettingTier,
    /// `hot` hoặc `restart`.
    pub apply: SettingApply,
    /// Nơi lưu.
    pub store: SettingStore,
    /// Mô tả tiếng Việt (ngôn ngữ mặc định của UI).
    pub desc_vi: &'static str,
    /// Mô tả tiếng Anh.
    pub desc_en: &'static str,
}

/// Bảng thiết lập duy nhất (M29 mục 4.2) — nguồn sự thật cho validate `config set`,
/// dựng UI và `bean config list`. Sắp theo đường dẫn cho dễ đọc; `lookup` duyệt tuyến
/// tính — danh sách ~50 dòng, không cần tìm kiếm nhị phân.
pub static SETTINGS: &[SettingEntry] = &[
    // ---- [llm] ----
    SettingEntry {
        path: "llm.model",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::SettingsState,
        desc_vi: "Model đang dùng của profile hiện tại (lựa chọn runtime)",
        desc_en: "Active model of the current profile (runtime selection)",
    },
    SettingEntry {
        path: "llm.provider",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Provider của profile `default` (anthropic | openai_compat)",
        desc_en: "Provider of the `default` profile (anthropic | openai_compat)",
    },
    SettingEntry {
        path: "llm.max_tokens",
        kind: SettingKind::U32,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Trần token cho mỗi lượt gọi LLM",
        desc_en: "Token cap per LLM call",
    },
    SettingEntry {
        path: "llm.base_url",
        kind: SettingKind::OptStr,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Base URL ghi đè (Ollama/gateway); bỏ trống để dùng mặc định",
        desc_en: "Override base URL (Ollama/gateway); empty = default",
    },
    SettingEntry {
        path: "llm.allowed_models",
        kind: SettingKind::StrList,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Danh sách model mà /model được phép chuyển",
        desc_en: "Models /model may switch to",
    },
    SettingEntry {
        path: "llm.api_key_env",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Tên biến môi trường chứa API key (giá trị key dùng `bean secret set`)",
        desc_en: "Env var name holding the API key (value via `bean secret set`)",
    },
    // ---- [tools] ----
    SettingEntry {
        path: "tools.enabled",
        kind: SettingKind::StrList,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Nhóm tool bật (files, shell, web, memory, skills, schedule, browser, billing, qa)",
        desc_en: "Enabled tool groups",
    },
    SettingEntry {
        path: "tools.web_search.provider",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Provider tìm kiếm: tavily | brave | searxng",
        desc_en: "Search provider: tavily | brave | searxng",
    },
    SettingEntry {
        path: "tools.web_search.base_url",
        kind: SettingKind::OptStr,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Base URL SearXNG tự host",
        desc_en: "Self-hosted SearXNG base URL",
    },
    SettingEntry {
        path: "tools.web_search.api_key_env",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Tên biến môi trường chứa key tìm kiếm",
        desc_en: "Env var name for the search key",
    },
    // ---- [telegram] ----
    SettingEntry {
        path: "telegram.enabled",
        kind: SettingKind::Bool,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Bật kênh Telegram (cần restart)",
        desc_en: "Enable the Telegram channel",
    },
    SettingEntry {
        path: "telegram.token_env",
        kind: SettingKind::Str,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Tên biến môi trường chứa bot token (giá trị: `bean secret set`, cần restart)",
        desc_en: "Env var name for the bot token (value via `bean secret set`)",
    },
    SettingEntry {
        path: "telegram.allowed_user_ids",
        kind: SettingKind::I64List,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Allowlist Telegram user id (can restart)",
        desc_en: "Telegram user id allowlist",
    },
    SettingEntry {
        path: "telegram.rate_limit_per_minute",
        kind: SettingKind::U32,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Giới hạn tin mỗi phút cho mỗi chat",
        desc_en: "Messages per minute per chat",
    },
    // ---- [agent] ----
    SettingEntry {
        path: "agent.timezone",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Múi giờ IANA cho cron và hiển thị",
        desc_en: "IANA timezone for cron and display",
    },
    SettingEntry {
        path: "agent.max_steps",
        kind: SettingKind::U32,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Trần số bước của một run",
        desc_en: "Max steps per run",
    },
    SettingEntry {
        path: "agent.context_budget_tokens",
        kind: SettingKind::U32,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Ngân sách token cho context gửi model",
        desc_en: "Token budget for the model context",
    },
    SettingEntry {
        path: "agent.allowed_users",
        kind: SettingKind::StrList,
        tier: SettingTier::B,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Lớp kiểm tra thứ hai ở lõi: user_id được dùng agent",
        desc_en: "Second-layer check: user_ids allowed to use the agent",
    },
    SettingEntry {
        path: "agent.workspace",
        kind: SettingKind::Str,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Thư mục làm việc của agent (path jail)",
        desc_en: "Agent workspace directory (path jail)",
    },
    // ---- [data] ----
    SettingEntry {
        path: "data.dir",
        kind: SettingKind::Str,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Thư mục dữ liệu: sqlite, secrets.env, auth",
        desc_en: "Data directory: sqlite, secrets.env, auth",
    },
    // ---- [security] ----
    SettingEntry {
        path: "security.daily_token_budget",
        kind: SettingKind::U64,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Ngân sách token mỗi ngày",
        desc_en: "Daily token budget",
    },
    SettingEntry {
        path: "security.tool_timeout_seconds",
        kind: SettingKind::U64,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Trần thời gian cho mỗi tool call (giây)",
        desc_en: "Per-tool-call timeout (seconds)",
    },
    SettingEntry {
        path: "security.sandbox.mode",
        kind: SettingKind::Str,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Sandbox shell: docker | host (host = mọi lệnh là Dangerous)",
        desc_en: "Sandbox shell: docker | host (host = every command Dangerous)",
    },
    SettingEntry {
        path: "security.sandbox.image",
        kind: SettingKind::Str,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Image sandbox Docker",
        desc_en: "Docker sandbox image",
    },
    SettingEntry {
        path: "security.sandbox.network",
        kind: SettingKind::Bool,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Cho container ra mạng",
        desc_en: "Allow container network access",
    },
    SettingEntry {
        path: "security.sandbox.memory",
        kind: SettingKind::Str,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Trần bộ nhớ container (vd 512m)",
        desc_en: "Container memory cap (e.g. 512m)",
    },
    SettingEntry {
        path: "security.sandbox.cpus",
        kind: SettingKind::F64,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Số CPU container được dùng",
        desc_en: "Container CPU count",
    },
    SettingEntry {
        path: "security.sandbox.pids_limit",
        kind: SettingKind::U32,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Trần tiến trình trong container (chống fork bomb)",
        desc_en: "Process cap inside container (anti fork bomb)",
    },
    SettingEntry {
        path: "security.sandbox.timeout_seconds",
        kind: SettingKind::U64,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Thời gian tối đa cho một lệnh shell (giây)",
        desc_en: "Max shell command time (seconds)",
    },
    // ---- [learning] ----
    SettingEntry {
        path: "learning.enabled",
        kind: SettingKind::Bool,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Bật learning loop (đề xuất skill sau run thành công)",
        desc_en: "Enable the learning loop (skill proposals)",
    },
    SettingEntry {
        path: "learning.min_tool_calls",
        kind: SettingKind::U32,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Số tool call tối thiểu trước khi reflection",
        desc_en: "Min tool calls before reflection",
    },
    SettingEntry {
        path: "learning.proposal_interval_minutes",
        kind: SettingKind::U64,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Khoảng cách tối thiểu giữa hai đề xuất (phút)",
        desc_en: "Min interval between proposals (minutes)",
    },
    // ---- [voice] — voice-state.json ----
    SettingEntry {
        path: "voice.enabled",
        kind: SettingKind::Bool,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::SettingsState,
        desc_vi: "Bật push-to-talk (cần cấu hình đủ [voice] mới thực sự hoạt động)",
        desc_en: "Enable push-to-talk (needs a complete [voice] config)",
    },
    SettingEntry {
        path: "voice.provider",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::SettingsState,
        desc_vi: "Nhà cung cấp STT/TTS: local | api",
        desc_en: "STT/TTS provider: local | api",
    },
    SettingEntry {
        path: "voice.language",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::SettingsState,
        desc_vi: "Ngôn ngữ STT/TTS (vi, en…)",
        desc_en: "STT/TTS language (vi, en…)",
    },
    SettingEntry {
        path: "voice.tts_voice_id",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::SettingsState,
        desc_vi: "Voice ID TTS mặc định (ElevenLabs)",
        desc_en: "Default TTS voice ID (ElevenLabs)",
    },
    SettingEntry {
        path: "voice.tts_voice_vi",
        kind: SettingKind::OptStr,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::SettingsState,
        desc_vi: "Voice ID TTS cho tiếng Việt; bỏ trống để xoá",
        desc_en: "TTS voice ID for Vietnamese; empty to clear",
    },
    // ---- [voice] — bean.toml ----
    SettingEntry {
        path: "voice.api_key_env",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Tên biến môi trường chứa key ElevenLabs (giá trị: `bean secret set`)",
        desc_en: "Env var name for the ElevenLabs key (value via `bean secret set`)",
    },
    SettingEntry {
        path: "voice.api_base_url",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Endpoint ElevenLabs (chỉ đổi khi tự host proxy)",
        desc_en: "ElevenLabs endpoint (only for a self-hosted proxy)",
    },
    SettingEntry {
        path: "voice.stt_model",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Model STT ElevenLabs (scribe_v2)",
        desc_en: "ElevenLabs STT model (scribe_v2)",
    },
    SettingEntry {
        path: "voice.tts_model",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Model TTS ElevenLabs (eleven_multilingual_v2)",
        desc_en: "ElevenLabs TTS model (eleven_multilingual_v2)",
    },
    SettingEntry {
        path: "voice.max_speak_chars",
        kind: SettingKind::U32,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Trần ký tự mỗi lần đọc TTS",
        desc_en: "Char cap per TTS utterance",
    },
    SettingEntry {
        path: "voice.timeout_seconds",
        kind: SettingKind::U64,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Thời gian tối đa cho mỗi lần STT/TTS (giây)",
        desc_en: "STT/TTS timeout (seconds)",
    },
    SettingEntry {
        path: "voice.fallback_voice_id",
        kind: SettingKind::OptStr,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Voice ID fallback khi giọng chính bị từ chối; bỏ trống để xoá",
        desc_en: "Fallback voice ID when the main one is rejected; empty to clear",
    },
    SettingEntry {
        path: "voice.whisper_command",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Lệnh whisper.cpp (chế độ local)",
        desc_en: "whisper.cpp command (local mode)",
    },
    SettingEntry {
        path: "voice.whisper_model",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "File model .bin của whisper.cpp",
        desc_en: "whisper.cpp .bin model file",
    },
    SettingEntry {
        path: "voice.piper_command",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "Lệnh piper (chế độ local)",
        desc_en: "piper command (local mode)",
    },
    SettingEntry {
        path: "voice.piper_model",
        kind: SettingKind::Str,
        tier: SettingTier::A,
        apply: SettingApply::Hot,
        store: SettingStore::BeanToml,
        desc_vi: "File model .onnx của Piper",
        desc_en: "Piper .onnx model file",
    },
    // ---- [web] ----
    SettingEntry {
        path: "web.enabled",
        kind: SettingKind::Bool,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Bật giao diện web",
        desc_en: "Enable the web UI",
    },
    SettingEntry {
        path: "web.bind",
        kind: SettingKind::Str,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Địa chỉ bind (mặc định 127.0.0.1:7878)",
        desc_en: "Bind address (default 127.0.0.1:7878)",
    },
    SettingEntry {
        path: "web.public_origin",
        kind: SettingKind::Str,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Origin công khai mà UI/WS phải khớp (chống CSRF/CSWSH)",
        desc_en: "Public origin the UI/WS must match (anti CSRF/CSWSH)",
    },
    SettingEntry {
        path: "web.allow_remote",
        kind: SettingKind::Bool,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Cho bind ngoài loopback (phải đặt sau reverse proxy TLS)",
        desc_en: "Allow non-loopback bind (must sit behind a TLS reverse proxy)",
    },
    SettingEntry {
        path: "web.trust_proxy",
        kind: SettingKind::Bool,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Tin X-Forwarded-For sau reverse proxy tin cậy",
        desc_en: "Trust X-Forwarded-For behind a trusted proxy",
    },
    SettingEntry {
        path: "web.session_ttl_hours",
        kind: SettingKind::U32,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "TTL phiên đăng nhập (giờ)",
        desc_en: "Login session TTL (hours)",
    },
    // ---- [browser] / [qa] ----
    SettingEntry {
        path: "browser.enabled",
        kind: SettingKind::Bool,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Bật nhóm tool browser nội bộ (CDP)",
        desc_en: "Enable the internal browser tool group (CDP)",
    },
    SettingEntry {
        path: "browser.executable_path",
        kind: SettingKind::Str,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Đường dẫn Chrome/Chromium; rỗng = tự phát hiện",
        desc_en: "Chrome/Chromium path; empty = auto-detect",
    },
    SettingEntry {
        path: "browser.allowed_origins",
        kind: SettingKind::StrList,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Origin được tool browser hành động ở mức Confirm (ngoài danh sách = Dangerous)",
        desc_en: "Origins allowed for Confirm-level browser actions (else Dangerous)",
    },
    SettingEntry {
        path: "qa.enabled",
        kind: SettingKind::Bool,
        tier: SettingTier::B,
        apply: SettingApply::Restart,
        store: SettingStore::BeanToml,
        desc_vi: "Bật nhóm tool QA (cần [[qa.suites]] mới chạy được)",
        desc_en: "Enable the QA tool group (needs [[qa.suites]] to run)",
    },
];

/// Tra cứu khoá trong bảng thiết lập; `None` khi khoá chưa được khai báo (config set
/// phải từ chối).
#[must_use]
pub fn lookup(path: &str) -> Option<&'static SettingEntry> {
    SETTINGS.iter().find(|entry| entry.path == path)
}

/// Parse chuỗi người dùng nhập thành [`SettingValue`] theo kiểu của khoá.
///
/// # Errors
/// [`SettingsError::InvalidSetting`] khi giá trị không khớp kiểu của khoá.
pub fn parse_value(entry: &SettingEntry, raw: &str) -> Result<SettingValue, SettingsError> {
    let key = entry.path;
    let invalid = |reason: String| SettingsError::InvalidSetting {
        key: key.to_string(),
        reason,
    };
    Ok(match entry.kind {
        SettingKind::Bool => match raw.trim().to_ascii_lowercase().as_str() {
            "true" => SettingValue::Bool(true),
            "false" => SettingValue::Bool(false),
            _ => return Err(invalid("cần `true` hoặc `false`".to_string())),
        },
        SettingKind::U32 => {
            let value: u32 = raw
                .trim()
                .parse()
                .map_err(|_| invalid(format!("cần số nguyên 0..={}", u32::MAX)))?;
            SettingValue::U32(value)
        }
        SettingKind::U64 => {
            const TOML_LIMIT: u64 = i64::MAX as u64;
            let limit = format!("cần số nguyên 0..={TOML_LIMIT} (giới hạn TOML)");
            let value: u64 = raw.trim().parse().map_err(|_| invalid(limit.clone()))?;
            if value > TOML_LIMIT {
                return Err(invalid(limit));
            }
            SettingValue::U64(value)
        }
        SettingKind::F64 => {
            let value: f64 = raw
                .trim()
                .parse()
                .map_err(|_| invalid("cần số thực, ví dụ 1.5".to_string()))?;
            if !value.is_finite() {
                return Err(invalid("phải là số hữu hạn".to_string()));
            }
            SettingValue::F64(value)
        }
        SettingKind::Str => SettingValue::Str(raw.trim().to_string()),
        SettingKind::OptStr => {
            let trimmed = raw.trim();
            SettingValue::OptStr(if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            })
        }
        SettingKind::StrList => SettingValue::StrList(parse_string_list(raw).map_err(invalid)?),
        SettingKind::I64List => SettingValue::I64List(parse_i64_list(raw).map_err(invalid)?),
    })
}

fn parse_string_list(raw: &str) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        return serde_json::from_str::<Vec<String>>(trimmed)
            .map_err(|error| format!("JSON không hợp lệ: {error}"));
    }
    Ok(trimmed
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect())
}

fn parse_i64_list(raw: &str) -> Result<Vec<i64>, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        return serde_json::from_str::<Vec<i64>>(trimmed)
            .map_err(|error| format!("JSON không hợp lệ: {error}"));
    }
    trimmed
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            item.parse::<i64>()
                .map_err(|_| format!("`{item}` không phải số nguyên"))
        })
        .collect()
}

/// Kết quả đọc một khoá: giá trị hiệu lực + nguồn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRead {
    /// Giá trị hiện có hiệu lực (đã gộp state runtime vào seed).
    pub value: String,
    /// Nguồn của giá trị.
    pub source: SettingSource,
}

/// Kết quả một lần ghi (set/unset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetOutcome {
    /// Nguồn trước khi ghi.
    pub old_source: SettingSource,
    /// Nguồn sau khi ghi.
    pub new_source: SettingSource,
    /// Khoá có cần restart không.
    pub apply: SettingApply,
}

/// Dịch vụ áp dụng thiết lập — CLI và web chỉ là adapter mỏng gọi vào đây (M29 mục
/// 4.3). Service tự đọc `bean.toml` + state files làm nguồn sự thật; người gọi chỉ cần
/// trỏ đúng `config_path`.
pub struct SettingsService {
    config_path: PathBuf,
    data_dir: PathBuf,
}

impl SettingsService {
    /// Dựng service từ cấu hình seed (lấy `data.dir`) và đường dẫn `bean.toml`.
    #[must_use]
    pub fn new(config: &Config, config_path: Option<&Path>) -> Self {
        Self {
            config_path: config_path
                .map_or_else(|| PathBuf::from(DEFAULT_CONFIG_FILE), Path::to_path_buf),
            data_dir: expand_home(&config.data.dir),
        }
    }

    /// Giá trị hiệu lực + nguồn của một khoá (đã gộp `llm-state`/`voice-state`).
    #[must_use]
    pub fn get(&self, entry: &SettingEntry) -> SettingRead {
        let seed = self.load_seed();
        let effective = self.effective(&seed);
        SettingRead {
            value: value_from_config(&effective, entry),
            source: self.source_of(entry),
        }
    }

    /// Ghi một khoá (validate trước khi ghi, audit sau khi ghi thành công).
    ///
    /// # Errors
    /// [`SettingsError::UnknownSetting`] không xảy ra ở tầng này (caller lookup trước);
    /// [`SettingsError::InvalidSetting`]/[`ConfigRejected`] khi giá trị hoặc cấu hình
    /// mới không đạt; [`SettingsError::Io`]/[`Toml`] khi đĩa lỗi; [`SettingsError::Audit`]
    /// khi không ghi được audit log (khi đó **không có gì** được ghi — audit là bắt buộc).
    pub fn set(
        &self,
        entry: &SettingEntry,
        value: SettingValue,
        audit: &AuditLog,
        actor: &str,
    ) -> Result<SetOutcome, SettingsError> {
        let old_source = self.source_of(entry);
        match entry.store {
            SettingStore::BeanToml => self.set_toml_key(entry, &value)?,
            SettingStore::SettingsState => {
                let seed = self.load_seed();
                self.set_settings_state(&seed, entry, &value)?;
            }
        }
        let new_source = self.source_of(entry);
        self.write_audit(audit, actor, entry, "config.set", old_source, new_source)?;
        Ok(SetOutcome {
            old_source,
            new_source,
            apply: entry.apply,
        })
    }

    /// Bỏ ghi đè của một khoá — về giá trị serde mặc định (toml) hoặc seed (state).
    ///
    /// # Errors
    /// Như [`Self::set`]. Unset một khoá chưa ghi đè là no-op (không ghi audit).
    pub fn unset(
        &self,
        entry: &SettingEntry,
        audit: &AuditLog,
        actor: &str,
    ) -> Result<SetOutcome, SettingsError> {
        let old_source = self.source_of(entry);
        let changed = match entry.store {
            SettingStore::BeanToml => self.unset_toml_key(entry)?,
            SettingStore::SettingsState => {
                let seed = self.load_seed();
                self.unset_settings_state(&seed, entry)?
            }
        };
        let new_source = self.source_of(entry);
        if changed {
            self.write_audit(audit, actor, entry, "config.unset", old_source, new_source)?;
        }
        Ok(SetOutcome {
            old_source,
            new_source,
            apply: entry.apply,
        })
    }

    fn settings_state_path(&self) -> PathBuf {
        self.data_dir.join(SETTINGS_STATE_FILE)
    }

    fn settings_state(&self) -> Option<SettingsState> {
        SettingsState::load(&self.settings_state_path())
    }

    /// Nạp seed từ `bean.toml`; file hỏng/thiếu ⇒ mặc định. Set/unset vẫn từ chối ghi
    /// khi file hỏng vì đi qua `toml_edit` parse.
    fn load_seed(&self) -> Config {
        let raw = match std::fs::read_to_string(&self.config_path) {
            Ok(raw) => raw,
            Err(_) => return Config::default(),
        };
        toml::from_str(&raw).unwrap_or_default()
    }

    /// Seed + state runtime — "giá trị hiệu lực" mà tiến trình sẽ thấy lúc khởi động.
    fn effective(&self, seed: &Config) -> Config {
        let mut config = seed.clone();
        // Giá trị trả về (có áp state hay không) không cần ở đây — chỉ cần config đã gộp.
        let _ = apply_persisted_settings_state(&mut config, &self.data_dir);
        config
    }

    fn source_of(&self, entry: &SettingEntry) -> SettingSource {
        let toml_source = || {
            if self.key_in_toml(entry.path) {
                SettingSource::BeanToml
            } else {
                SettingSource::Default
            }
        };
        match entry.store {
            SettingStore::BeanToml => toml_source(),
            SettingStore::SettingsState => {
                // State áp cả llm (profile/model) lẫn voice (enabled/provider/language/voice id)
                // nên khi file tồn tại, mọi khoá liên quan đều coi như đến từ state.
                if self.settings_state().is_some() {
                    SettingSource::SettingsState
                } else {
                    toml_source()
                }
            }
        }
    }

    fn key_in_toml(&self, path: &str) -> bool {
        let Ok(mut doc) = self.load_doc() else {
            return false;
        };
        let mut parts: Vec<&str> = path.split('.').collect();
        let Some(key) = parts.pop() else { return false };
        let Some(table) = existing_table(doc.as_table_mut(), &parts) else {
            return false;
        };
        table.contains_key(key)
    }

    fn load_doc(&self) -> Result<toml_edit::DocumentMut, SettingsError> {
        let raw = match std::fs::read_to_string(&self.config_path) {
            Ok(raw) => raw,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(SettingsError::Io {
                    path: self.config_path.clone(),
                    source,
                });
            }
        };
        raw.parse::<toml_edit::DocumentMut>()
            .map_err(|error| SettingsError::Toml {
                path: self.config_path.clone(),
                reason: error.to_string(),
            })
    }

    fn write_doc(&self, doc: &toml_edit::DocumentMut) -> Result<(), SettingsError> {
        let out = doc.to_string();
        if let Some(parent) = self.config_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| SettingsError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let tmp = self.config_path.with_extension("toml.tmp");
        std::fs::write(&tmp, out.as_bytes()).map_err(|source| SettingsError::Io {
            path: tmp.clone(),
            source,
        })?;
        // Giữ nguyên quyền của file gốc (thường 0644); file mới để umask quyết định.
        #[cfg(unix)]
        if let Ok(meta) = std::fs::metadata(&self.config_path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        std::fs::rename(&tmp, &self.config_path).map_err(|source| SettingsError::Io {
            path: self.config_path.clone(),
            source,
        })?;
        Ok(())
    }

    /// Ghi một khoá vào `bean.toml`: sửa bằng `toml_edit`, parse lại toàn bộ file
    /// thành `Config` + `validate()` — chỉ ghi khi hợp lệ.
    fn set_toml_key(
        &self,
        entry: &SettingEntry,
        value: &SettingValue,
    ) -> Result<(), SettingsError> {
        let mut doc = self.load_doc()?;
        let mut parts: Vec<&str> = entry.path.split('.').collect();
        let Some(key) = parts.pop() else {
            return Err(SettingsError::InvalidSetting {
                key: entry.path.to_string(),
                reason: "đường dẫn khoá rỗng".to_string(),
            });
        };
        let table =
            nested_table(doc.as_table_mut(), &parts).ok_or_else(|| SettingsError::Toml {
                path: self.config_path.clone(),
                reason: format!("không tạo được bảng cho `{}`", parts.join(".")),
            })?;
        if let Some(mut new_item) = toml_item(value) {
            // `insert` thay cả item — chuyển decor của giá trị cũ (khoảng trắng và
            // comment cuối dòng) sang giá trị mới, nếu không người dùng mất comment
            // ngay trên dòng bị sửa. Decor của key (comment đứng trên) không đụng tới.
            if let Some(new_value) = new_item.as_value_mut()
                && let Some(old_value) = table.get(key).and_then(toml_edit::Item::as_value)
            {
                let old_decor = old_value.decor();
                if let Some(prefix) = old_decor.prefix() {
                    new_value.decor_mut().set_prefix(prefix.clone());
                }
                if let Some(suffix) = old_decor.suffix() {
                    new_value.decor_mut().set_suffix(suffix.clone());
                }
            }
            table.insert(key, new_item);
        } else {
            table.remove(key);
        }
        // Validate file sau khi sửa — cấu hình sai bị từ chối ngay, không đợi lần
        // khởi động sau mới chết (M29 mục 4.3).
        let mut new_config: Config =
            toml::from_str(&doc.to_string()).map_err(|error| SettingsError::Toml {
                path: self.config_path.clone(),
                reason: error.to_string(),
            })?;
        new_config
            .validate()
            .map_err(|error| SettingsError::ConfigRejected {
                message: error.to_string(),
            })?;
        self.write_doc(&doc)
    }

    /// Xoá một khoá khỏi `bean.toml` (về giá trị serde mặc định). Trả `false` khi khoá
    /// chưa có trong file — không ghi gì.
    fn unset_toml_key(&self, entry: &SettingEntry) -> Result<bool, SettingsError> {
        let mut doc = self.load_doc()?;
        let mut parts: Vec<&str> = entry.path.split('.').collect();
        let Some(key) = parts.pop() else {
            return Ok(false);
        };
        let Some(table) = existing_table(doc.as_table_mut(), &parts) else {
            return Ok(false);
        };
        if table.remove(key).is_none() {
            return Ok(false);
        }
        let mut new_config: Config =
            toml::from_str(&doc.to_string()).map_err(|error| SettingsError::Toml {
                path: self.config_path.clone(),
                reason: error.to_string(),
            })?;
        new_config
            .validate()
            .map_err(|error| SettingsError::ConfigRejected {
                message: error.to_string(),
            })?;
        self.write_doc(&doc)?;
        Ok(true)
    }

    fn set_settings_state(
        &self,
        seed: &Config,
        entry: &SettingEntry,
        value: &SettingValue,
    ) -> Result<(), SettingsError> {
        let path = self.settings_state_path();
        let mut state = self.settings_state().unwrap_or_else(|| SettingsState {
            llm_profile: seed.llm.active_profile_name().to_string(),
            llm_model: seed.llm.model.clone(),
            voice_enabled: seed.voice.enabled,
            voice_provider: None,
            voice_language: None,
            voice_tts_voice_id: None,
            voice_tts_voice_vi: None,
        });
        match (entry.path, value) {
            ("llm.model", SettingValue::Str(model)) => {
                let profile = seed.llm.profile(&state.llm_profile).ok_or_else(|| {
                    SettingsError::ConfigRejected {
                        message: format!(
                            "profile `{}` trong settings-state.json không còn trong bean.toml — chọn lại profile trước khi đổi model",
                            state.llm_profile
                        ),
                    }
                })?;
                let allowed = profile.effective_allowed_models();
                if !allowed.iter().any(|candidate| candidate == model) {
                    return Err(SettingsError::InvalidSetting {
                        key: entry.path.to_string(),
                        reason: format!(
                            "model phải nằm trong allowed_models của profile `{}`: [{}]",
                            state.llm_profile,
                            allowed.join(", ")
                        ),
                    });
                }
                state.llm_model = model.clone();
            }
            ("voice.enabled", SettingValue::Bool(enabled)) => state.voice_enabled = *enabled,
            ("voice.provider", SettingValue::Str(raw)) => {
                let parsed = SpeechProviderKind::from_config_str(raw).ok_or_else(|| {
                    SettingsError::InvalidSetting {
                        key: entry.path.to_string(),
                        reason: "cần `local` hoặc `api`".to_string(),
                    }
                })?;
                state.voice_provider = Some(parsed);
            }
            ("voice.language", SettingValue::Str(language)) => {
                state.voice_language = Some(language.clone());
            }
            ("voice.tts_voice_id", SettingValue::Str(id)) => {
                state.voice_tts_voice_id = Some(id.clone());
            }
            ("voice.tts_voice_vi", SettingValue::Str(id) | SettingValue::OptStr(Some(id))) => {
                state.voice_tts_voice_vi = Some(id.clone());
            }
            ("voice.tts_voice_vi", SettingValue::OptStr(None)) => state.voice_tts_voice_vi = None,
            _ => {
                return Err(SettingsError::InvalidSetting {
                    key: entry.path.to_string(),
                    reason: "khoá này không lưu ở settings-state.json hoặc kiểu giá trị sai"
                        .to_string(),
                });
            }
        }
        // Chặn "bật có nghĩa" trước khi ghi: state bật voice nhưng `[voice]` chưa đủ
        // cấu hình thì `apply_to_config` sẽ giữ seed — người dùng bật mà không thấy
        // gì xảy ra. Từ chối ngay với lý do rõ ràng.
        let mut probe = seed.clone();
        let applied = state.apply_to_config(&mut probe);
        if state.voice_enabled && !applied {
            return Err(SettingsError::ConfigRejected {
                message: "[voice] chưa đủ cấu hình để bật — chế độ local cần whisper_command/whisper_model và piper_command/piper_model, chế độ api cần key; hoàn tất [voice] trong bean.toml trước".to_string(),
            });
        }
        SettingsState::save(&path, &state).map_err(|source| SettingsError::Io { path, source })?;
        Ok(())
    }

    /// Unset một trường settings-state; khi state không còn ghi đè gì thì xoá file —
    /// đúng ngữ nghĩa "vắng mặt ⇒ theo seed".
    fn unset_settings_state(
        &self,
        seed: &Config,
        entry: &SettingEntry,
    ) -> Result<bool, SettingsError> {
        let path = self.settings_state_path();
        let Some(mut state) = self.settings_state() else {
            return Ok(false);
        };
        match entry.path {
            "llm.model" => state.llm_model = seed.llm.model.clone(),
            "voice.enabled" => state.voice_enabled = seed.voice.enabled,
            "voice.provider" => state.voice_provider = None,
            "voice.language" => state.voice_language = None,
            "voice.tts_voice_id" => state.voice_tts_voice_id = None,
            "voice.tts_voice_vi" => state.voice_tts_voice_vi = None,
            _ => return Ok(false),
        }
        let seed_state = SettingsState {
            llm_profile: seed.llm.active_profile_name().to_string(),
            llm_model: seed.llm.model.clone(),
            voice_enabled: seed.voice.enabled,
            voice_provider: None,
            voice_language: None,
            voice_tts_voice_id: None,
            voice_tts_voice_vi: None,
        };
        if state == seed_state {
            remove_if_exists(&path)?;
        } else {
            SettingsState::save(&path, &state).map_err(|source| SettingsError::Io {
                path: path.clone(),
                source,
            })?;
        }
        Ok(true)
    }

    /// Audit một lần ghi — chỉ tên khoá + nguồn cũ→mới, **không bao giờ** giá trị.
    fn write_audit(
        &self,
        audit: &AuditLog,
        actor: &str,
        entry: &SettingEntry,
        tool: &'static str,
        old_source: SettingSource,
        new_source: SettingSource,
    ) -> Result<(), SettingsError> {
        // Tên arg là `setting`, KHÔNG phải `key`: redactor của audit khớp theo
        // `contains("key")` nên arg `key` sẽ bị che thành `[REDACTED]` — mất thông tin
        // mà bản thân nó không hề nhạy cảm (chỉ là đường dẫn khoá).
        let args = json!({
            "setting": entry.path,
            "tier": entry.tier.short(),
            "old": old_source.label(),
            "new": new_source.label(),
            "actor": actor,
        });
        let mut record = entry_now(0, "cli", tool, &args);
        record.ok = Some(true);
        record.decision = "applied";
        record.decided_by = actor.to_string();
        audit.record(&record).map_err(|error| SettingsError::Audit {
            message: error.to_string(),
        })
    }
}

fn remove_if_exists(path: &Path) -> Result<(), SettingsError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SettingsError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Chuyển [`SettingValue`] thành item TOML; `None` khi giá trị nghĩa là "xoá khoá"
/// (chỉ [`SettingValue::OptStr`] với `None`).
fn toml_item(value: &SettingValue) -> Option<toml_edit::Item> {
    Some(match value {
        SettingValue::Bool(v) => toml_edit::value(*v),
        SettingValue::U32(v) => toml_edit::value(i64::from(*v)),
        // Bất biến: `parse_value` chặn U64 > i64::MAX nên cast không tràn.
        SettingValue::U64(v) => toml_edit::value(*v as i64),
        SettingValue::F64(v) => toml_edit::value(*v),
        SettingValue::Str(v) | SettingValue::OptStr(Some(v)) => toml_edit::value(v.as_str()),
        SettingValue::OptStr(None) => return None,
        SettingValue::StrList(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(item.as_str());
            }
            toml_edit::value(array)
        }
        SettingValue::I64List(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(*item);
            }
            toml_edit::value(array)
        }
    })
}

/// Điều hướng (tạo nếu thiếu) tới bảng con tương ứng các section; key cuối do caller
/// xử lý. Trả `None` khi một tầng nào đó không thể là bảng (lỗi hi hữu của toml_edit).
fn nested_table<'a>(
    root: &'a mut dyn toml_edit::TableLike,
    sections: &[&str],
) -> Option<&'a mut dyn toml_edit::TableLike> {
    let Some((first, rest)) = sections.split_first() else {
        return Some(root);
    };
    let item = root.entry(first).or_insert(toml_edit::table());
    // Bảng hiện có (kể cả inline table người dùng viết gọn) dùng được luôn — giữ
    // nguyên định dạng. Chỉ thay khi item là giá trị khác (chuỗi/mảng) lồng sai chỗ.
    if item.as_table_like_mut().is_none() {
        *item = toml_edit::table();
    }
    let next = item.as_table_like_mut()?;
    nested_table(next, rest)
}

/// Điều hướng **chỉ đọc** tới bảng con; `None` khi đường dẫn chưa tồn tại (unset một
/// khoá chưa ghi không được tự tạo bảng).
fn existing_table<'a>(
    root: &'a mut dyn toml_edit::TableLike,
    sections: &[&str],
) -> Option<&'a mut dyn toml_edit::TableLike> {
    let Some((first, rest)) = sections.split_first() else {
        return Some(root);
    };
    let item = root.get_mut(first)?;
    let table = item.as_table_like_mut()?;
    existing_table(table, rest)
}

fn search_provider_str(provider: bean_types::config::WebSearchProvider) -> &'static str {
    use bean_types::config::WebSearchProvider;
    match provider {
        WebSearchProvider::Tavily => "tavily",
        WebSearchProvider::Brave => "brave",
        WebSearchProvider::Searxng => "searxng",
    }
}

/// Giá trị của một khoá đọc từ `Config` (đã gộp state) — bảng và hàm này phải khớp
/// nhau: mọi dòng trong [`SETTINGS`] phải có nhánh ở đây (test bảng kiểm tra).
fn value_from_config(config: &Config, entry: &SettingEntry) -> String {
    match entry.path {
        // ---- [llm] ----
        "llm.model" => config.llm.model.clone(),
        "llm.provider" => config.llm.provider.as_str().to_string(),
        "llm.max_tokens" => config.llm.max_tokens.to_string(),
        "llm.base_url" => config.llm.base_url.clone().unwrap_or_default(),
        "llm.allowed_models" => format!("[{}]", config.llm.allowed_models.join(", ")),
        "llm.api_key_env" => config.llm.api_key_env.clone(),
        // ---- [tools] ----
        "tools.enabled" => format!("[{}]", config.tools.enabled.join(", ")),
        "tools.web_search.provider" => {
            search_provider_str(config.tools.web_search.provider).to_string()
        }
        "tools.web_search.base_url" => config.tools.web_search.base_url.clone().unwrap_or_default(),
        "tools.web_search.api_key_env" => config.tools.web_search.api_key_env.clone(),
        // ---- [telegram] ----
        "telegram.enabled" => config.telegram.enabled.to_string(),
        "telegram.token_env" => config.telegram.token_env.clone(),
        "telegram.allowed_user_ids" => {
            let rendered: Vec<String> = config
                .telegram
                .allowed_user_ids
                .iter()
                .map(i64::to_string)
                .collect();
            format!("[{}]", rendered.join(", "))
        }
        "telegram.rate_limit_per_minute" => config.telegram.rate_limit_per_minute.to_string(),
        // ---- [agent] / [data] ----
        "agent.timezone" => config.agent.timezone.clone(),
        "agent.max_steps" => config.agent.max_steps.to_string(),
        "agent.context_budget_tokens" => config.agent.context_budget_tokens.to_string(),
        "agent.allowed_users" => format!("[{}]", config.agent.allowed_users.join(", ")),
        "agent.workspace" => config.agent.workspace.display().to_string(),
        "data.dir" => config.data.dir.display().to_string(),
        // ---- [security] ----
        "security.daily_token_budget" => config.security.daily_token_budget.to_string(),
        "security.tool_timeout_seconds" => config.security.tool_timeout_seconds.to_string(),
        "security.sandbox.mode" => config.security.sandbox.mode.as_str().to_string(),
        "security.sandbox.image" => config.security.sandbox.image.clone(),
        "security.sandbox.network" => config.security.sandbox.network.to_string(),
        "security.sandbox.memory" => config.security.sandbox.memory.clone(),
        "security.sandbox.cpus" => config.security.sandbox.cpus.to_string(),
        "security.sandbox.pids_limit" => config.security.sandbox.pids_limit.to_string(),
        "security.sandbox.timeout_seconds" => config.security.sandbox.timeout_seconds.to_string(),
        // ---- [learning] ----
        "learning.enabled" => config.learning.enabled.to_string(),
        "learning.min_tool_calls" => config.learning.min_tool_calls.to_string(),
        "learning.proposal_interval_minutes" => {
            config.learning.proposal_interval_minutes.to_string()
        }
        // ---- [voice] ----
        "voice.enabled" => config.voice.enabled.to_string(),
        "voice.provider" => config.voice.provider.as_str().to_string(),
        "voice.language" => config.voice.language.clone(),
        "voice.tts_voice_id" => config.voice.tts_voice_id.clone(),
        "voice.tts_voice_vi" => config.voice.tts_voice_vi.clone().unwrap_or_default(),
        "voice.api_key_env" => config.voice.api_key_env.clone(),
        "voice.api_base_url" => config.voice.api_base_url.clone(),
        "voice.stt_model" => config.voice.stt_model.clone(),
        "voice.tts_model" => config.voice.tts_model.clone(),
        "voice.max_speak_chars" => config.voice.max_speak_chars.to_string(),
        "voice.timeout_seconds" => config.voice.timeout_seconds.to_string(),
        "voice.fallback_voice_id" => config.voice.fallback_voice_id.clone().unwrap_or_default(),
        "voice.whisper_command" => config.voice.whisper_command.clone(),
        "voice.whisper_model" => config.voice.whisper_model.clone(),
        "voice.piper_command" => config.voice.piper_command.clone(),
        "voice.piper_model" => config.voice.piper_model.clone(),
        // ---- [web] ----
        "web.enabled" => config.web.enabled.to_string(),
        "web.bind" => config.web.bind.to_string(),
        "web.public_origin" => config.web.public_origin.clone(),
        "web.allow_remote" => config.web.allow_remote.to_string(),
        "web.trust_proxy" => config.web.trust_proxy.to_string(),
        "web.session_ttl_hours" => config.web.session_ttl_hours.to_string(),
        // ---- [browser] / [qa] ----
        "browser.enabled" => config.browser.enabled.to_string(),
        "browser.executable_path" => config.browser.executable_path.clone(),
        "browser.allowed_origins" => {
            format!("[{}]", config.browser.allowed_origins.join(", "))
        }
        "qa.enabled" => config.qa.enabled.to_string(),
        // Bất biến: test `every_schema_entry_has_a_value_branch` bắt nhánh thiếu.
        _ => format!("<không có nhánh: {}>", entry.path),
    }
}

/// Mở rộng `~` đầu đường dẫn — bản nhỏ của `expand_tilde` (bean-types không expose).
fn expand_home(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    } else if text == "~"
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use bean_types::config::{Config, SettingsState};

    /// Ghi nội dung `bean.toml` vào thư mục tạm, tự nối `[data] dir` trỏ vào tempdir
    /// để state/audit không bao giờ rơi vào `$HOME` thật. Nội dung test không được
    /// chứa `[data]`.
    fn write_config(dir: &tempfile::TempDir, content: &str) -> PathBuf {
        let path = dir.path().join("bean.toml");
        let full = format!("{content}\n[data]\ndir = \"{}\"\n", dir.path().display());
        std::fs::write(&path, full).unwrap();
        path
    }

    fn service(config_path: &Path) -> SettingsService {
        let seed = Config::load(config_path).unwrap();
        // `data.dir` trong file trỏ vào tempdir — service tự đọc đúng chỗ đó.
        SettingsService::new(&seed, Some(config_path))
    }

    #[test]
    fn unknown_key_is_not_in_schema() {
        assert!(lookup("khong.co.tren.troi").is_none());
        assert!(lookup("PATH").is_none());
        assert!(lookup("user_roles").is_none());
        assert!(lookup("llm.model").is_some());
        assert!(lookup("security.sandbox.cpus").is_some());
    }

    #[test]
    fn parse_value_kinds() {
        let entry = lookup("telegram.enabled").unwrap();
        assert_eq!(
            parse_value(entry, "true").unwrap(),
            SettingValue::Bool(true)
        );
        assert_eq!(
            parse_value(entry, " FALSE ").unwrap(),
            SettingValue::Bool(false)
        );
        assert!(parse_value(entry, "co").is_err());

        let entry = lookup("agent.max_steps").unwrap();
        assert_eq!(parse_value(entry, "42").unwrap(), SettingValue::U32(42));
        assert!(parse_value(entry, "x42").is_err());

        let entry = lookup("security.daily_token_budget").unwrap();
        assert_eq!(
            parse_value(entry, "5000000").unwrap(),
            SettingValue::U64(5_000_000)
        );
        assert!(
            parse_value(entry, "18446744073709551615").is_err(),
            "vượt i64::MAX bị chặn"
        );

        let entry = lookup("telegram.allowed_user_ids").unwrap();
        assert_eq!(
            parse_value(entry, "[1,2]").unwrap(),
            SettingValue::I64List(vec![1, 2])
        );
        assert_eq!(
            parse_value(entry, "1, 2").unwrap(),
            SettingValue::I64List(vec![1, 2])
        );
        assert!(parse_value(entry, "1, hai").is_err());

        let entry = lookup("tools.enabled").unwrap();
        assert_eq!(
            parse_value(entry, r#"["files", "web"]"#).unwrap(),
            SettingValue::StrList(vec!["files".into(), "web".into()])
        );
        assert_eq!(
            parse_value(entry, "files, web").unwrap(),
            SettingValue::StrList(vec!["files".into(), "web".into()])
        );

        let entry = lookup("llm.base_url").unwrap();
        assert_eq!(parse_value(entry, "").unwrap(), SettingValue::OptStr(None));
        assert_eq!(
            parse_value(entry, " http://localhost:11434/v1 ").unwrap(),
            SettingValue::OptStr(Some("http://localhost:11434/v1".into()))
        );

        let entry = lookup("security.sandbox.cpus").unwrap();
        assert_eq!(parse_value(entry, "2.5").unwrap(), SettingValue::F64(2.5));
    }

    #[test]
    fn set_writes_toml_and_keeps_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(
            &dir,
            "# cấu hình của tôi\n[agent]\nmax_steps = 10 # giữ cmt\n",
        );
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        let entry = lookup("agent.timezone").unwrap();
        let value = parse_value(entry, "Europe/Paris").unwrap();
        let outcome = store.set(entry, value, &audit, "test:actor").unwrap();
        assert_eq!(outcome.apply, SettingApply::Hot);
        assert_eq!(outcome.new_source, SettingSource::BeanToml);

        // Sửa tiếp chính khoá có comment cuối dòng — comment phải còn nguyên.
        let entry = lookup("agent.max_steps").unwrap();
        let value = parse_value(entry, "60").unwrap();
        store.set(entry, value, &audit, "test:actor").unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("# cấu hình của tôi"),
            "comment đầu file phải giữ nguyên"
        );
        assert!(
            raw.contains("# giữ cmt"),
            "comment cuối dòng phải giữ nguyên"
        );
        assert!(
            raw.contains("max_steps = 60 # giữ cmt"),
            "comment phải sống sót qua việc sửa chính khoá đó"
        );
        assert!(raw.contains("timezone = \"Europe/Paris\""));

        let reloaded = Config::load(&path).unwrap();
        assert_eq!(reloaded.agent.timezone, "Europe/Paris");
        assert_eq!(reloaded.agent.max_steps, 60);
    }

    #[test]
    fn set_rejects_value_that_fails_validate_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, "[tools]\nenabled = [\"files\"]\n");
        let before = std::fs::read_to_string(&path).unwrap();
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        let entry = lookup("tools.enabled").unwrap();
        let value = parse_value(entry, "files, khong-ton-tai").unwrap();
        let result = store.set(entry, value, &audit, "test:actor");
        assert!(result.is_err(), "validate phải từ chối nhóm tool lạ");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "file không được ghi"
        );
    }

    #[test]
    fn set_rejects_unparsable_toml_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, "");
        let before = std::fs::read_to_string(&path).unwrap();
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        let entry = lookup("web.bind").unwrap();
        let value = parse_value(entry, "khong-phai-socket-addr").unwrap();
        assert!(store.set(entry, value, &audit, "test:actor").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn set_llm_model_writes_state_and_rejects_outside_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(
            &dir,
            "[llm]\nmodel = \"m1\"\nallowed_models = [\"m1\", \"m2\"]\n",
        );
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        let entry = lookup("llm.model").unwrap();
        let value = parse_value(entry, "m2").unwrap();
        let outcome = store.set(entry, value, &audit, "test:actor").unwrap();
        assert_eq!(outcome.new_source, SettingSource::SettingsState);
        assert_eq!(
            SettingsState::load(&dir.path().join(SETTINGS_STATE_FILE)),
            Some(SettingsState {
                llm_profile: "default".into(),
                llm_model: "m2".into(),
                voice_enabled: false,
                voice_provider: None,
                voice_language: None,
                voice_tts_voice_id: None,
                voice_tts_voice_vi: None,
            })
        );
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("model = \"m2\""),
            "llm.model là lựa chọn runtime — không được ghi vào bean.toml"
        );

        let value = parse_value(entry, "m3").unwrap();
        assert!(store.set(entry, value, &audit, "test:actor").is_err());
        assert_eq!(
            SettingsState::load(&dir.path().join(SETTINGS_STATE_FILE)),
            Some(SettingsState {
                llm_profile: "default".into(),
                llm_model: "m2".into(),
                voice_enabled: false,
                voice_provider: None,
                voice_language: None,
                voice_tts_voice_id: None,
                voice_tts_voice_vi: None,
            })
        );
    }

    #[test]
    fn unset_restores_default_and_removes_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, "[agent]\nmax_steps = 10\n");
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        let entry = lookup("agent.max_steps").unwrap();
        let value = parse_value(entry, "50").unwrap();
        store.set(entry, value, &audit, "test:actor").unwrap();
        let read = store.get(entry);
        assert_eq!(read.value, "50");
        assert_eq!(read.source, SettingSource::BeanToml);

        store.unset(entry, &audit, "test:actor").unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("max_steps")
        );
        let read = store.get(entry);
        assert_eq!(read.value, "25");
        assert_eq!(read.source, SettingSource::Default);
    }

    #[test]
    fn unset_untouched_key_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, "[agent]\nmax_steps = 10\n");
        let before = std::fs::read_to_string(&path).unwrap();
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        // `agent.timezone` không có trong file — unset không ghi gì.
        let entry = lookup("agent.timezone").unwrap();
        let outcome = store.unset(entry, &audit, "test:actor").unwrap();
        assert_eq!(outcome.old_source, SettingSource::Default);
        assert_eq!(outcome.new_source, SettingSource::Default);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn voice_state_set_get_unset_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(
            &dir,
            // Seed `[voice]` đủ cấu hình local — thiếu model thì `voice_configured()`
            // = false và bật state bị từ chối, không test được roundtrip.
            "[voice]\nenabled = false\nwhisper_model = \"/models/ggml-base.bin\"\npiper_model = \"/models/vi_VN-huynh-medium.onnx\"\n",
        );
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        let enabled = lookup("voice.enabled").unwrap();
        let vi = lookup("voice.tts_voice_vi").unwrap();

        let read = store.get(enabled);
        assert_eq!(read.value, "false");
        assert_eq!(read.source, SettingSource::BeanToml);

        let value = parse_value(enabled, "true").unwrap();
        store.set(enabled, value, &audit, "test:actor").unwrap();
        let read = store.get(enabled);
        assert_eq!(read.value, "true");
        assert_eq!(read.source, SettingSource::SettingsState);

        let value = parse_value(vi, "giọng-việt").unwrap();
        store.set(vi, value, &audit, "test:actor").unwrap();
        let read = store.get(vi);
        assert_eq!(read.value, "giọng-việt");
        assert_eq!(read.source, SettingSource::SettingsState);

        // Unset từng trường; state còn tts_voice_vi chưa xoá nên file vẫn còn.
        store.unset(enabled, &audit, "test:actor").unwrap();
        assert!(dir.path().join(SETTINGS_STATE_FILE).exists());
        store.unset(vi, &audit, "test:actor").unwrap();
        assert!(
            !dir.path().join(SETTINGS_STATE_FILE).exists(),
            "state về đúng seed — file phải bị xoá"
        );
        let read = store.get(enabled);
        assert_eq!(read.value, "false");
        assert_eq!(read.source, SettingSource::BeanToml);
    }

    #[test]
    fn enabling_voice_without_complete_section_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // Seed thiếu whisper_model/piper_model — `voice_configured()` = false.
        let path = write_config(&dir, "[voice]\nenabled = false\n");
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        let enabled = lookup("voice.enabled").unwrap();
        let value = parse_value(enabled, "true").unwrap();
        let error = store
            .set(enabled, value, &audit, "test:actor")
            .expect_err("bật voice thiếu cấu hình phải bị từ chối");
        assert!(matches!(error, SettingsError::ConfigRejected { .. }));
        assert!(
            !dir.path().join(SETTINGS_STATE_FILE).exists(),
            "không được ghi state khi bị từ chối"
        );
    }

    #[test]
    fn unset_llm_model_removes_state_when_matching_seed() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(
            &dir,
            "[llm]\nmodel = \"m1\"\nallowed_models = [\"m1\", \"m2\"]\n",
        );
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        let entry = lookup("llm.model").unwrap();
        let value = parse_value(entry, "m2").unwrap();
        store.set(entry, value, &audit, "test:actor").unwrap();
        assert!(dir.path().join(SETTINGS_STATE_FILE).exists());

        store.unset(entry, &audit, "test:actor").unwrap();
        assert!(
            !dir.path().join(SETTINGS_STATE_FILE).exists(),
            "state trùng seed — file phải bị xoá"
        );
        let read = store.get(entry);
        assert_eq!(read.value, "m1");
        assert_eq!(read.source, SettingSource::BeanToml);
    }

    #[test]
    fn audit_contains_no_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, "");
        let store = service(&path);
        let audit = AuditLog::open(dir.path()).unwrap();

        let entry = lookup("agent.timezone").unwrap();
        let value = parse_value(entry, "Europe/Paris").unwrap();
        store.set(entry, value, &audit, "test:actor").unwrap();

        let content = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert!(
            content.contains("\"setting\":\"agent.timezone\""),
            "{content}"
        );
        assert!(content.contains("config.set"), "{content}");
        assert!(
            !content.contains("Europe/Paris"),
            "audit không được chứa giá trị: {content}"
        );
    }

    #[test]
    fn every_schema_entry_has_a_value_branch() {
        for entry in SETTINGS {
            let value = value_from_config(&Config::default(), entry);
            assert!(
                !value.starts_with("<không có nhánh"),
                "{} thiếu nhánh trong value_from_config",
                entry.path
            );
        }
    }

    #[test]
    fn every_entry_is_reachable_via_lookup_and_paths_are_unique() {
        // Bảng giữ thứ tự nhóm theo section cho dễ bảo trì; thứ tự hiển thị do CLI
        // sort khi in. Bất biến cần giữ là: path duy nhất + lookup tìm được mọi dòng.
        let mut paths: Vec<&str> = SETTINGS.iter().map(|entry| entry.path).collect();
        let total = paths.len();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), total, "path trong bảng phải duy nhất");
        for path in paths {
            assert!(lookup(path).is_some(), "lookup phải tìm được {path}");
        }
    }

    #[test]
    fn schema_covers_tier_b_security_keys() {
        // D19.4: các khoá bảo mật bắt buộc phải là tầng B.
        let must_be_b = [
            "security.sandbox.mode",
            "web.bind",
            "web.public_origin",
            "agent.workspace",
            "data.dir",
            "browser.allowed_origins",
        ];
        for path in must_be_b {
            let entry = lookup(path).expect("khoá bảo mật phải có trong bảng");
            assert_eq!(entry.tier, SettingTier::B, "{path} phải là tầng B");
        }
    }
}
