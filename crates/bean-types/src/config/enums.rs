//! Các enum nhỏ dùng chung cho nhiều section (`LlmProviderKind`, `SandboxMode`, ...).

use serde::{Deserialize, Serialize};

/// Provider LLM (`[llm] provider`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderKind {
    /// Anthropic Messages API.
    Anthropic,
    /// Mọi endpoint tương thích OpenAI Chat Completions (OpenAI, Ollama, vLLM…).
    // `snake_case` của variant này là `open_ai_compat` — đặt tên tường minh để khớp
    // `as_str()` và `bean.example.toml`.
    #[serde(rename = "openai_compat")]
    OpenAiCompat,
}

impl LlmProviderKind {
    /// Chuỗi như trong file cấu hình.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAiCompat => "openai_compat",
        }
    }
}

/// Chế độ sandbox cho `run_shell` (agents.md mục 15.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Mặc định: `docker run --rm` với workspace mount, không mạng, non-root.
    Docker,
    /// Chạy thẳng trên host — phải bật tường minh, mọi lệnh là `Dangerous`.
    Host,
}

impl SandboxMode {
    /// Chuỗi như trong file cấu hình.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Host => "host",
        }
    }
}

/// Provider cho `web_search` (agents.md mục 7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchProvider {
    /// Tavily.
    Tavily,
    /// Brave Search.
    Brave,
    /// SearXNG tự host (không cần API key).
    Searxng,
}

impl WebSearchProvider {
    /// Provider này có cần API key không?
    #[must_use]
    pub const fn requires_api_key(self) -> bool {
        !matches!(self, Self::Searxng)
    }
}

/// Kiểu target trong `[[infra_scope]]` (M23).
///
/// **Chỉ chấp nhận `ip`/`cidr`**, cố ý **không** có `hostname`: nếu cho phép tên miền thì
/// code kiểm scope và scanner sẽ phân giải DNS ở hai thời điểm khác nhau, tạo khe hở TOCTOU
/// (check ra IP được phép, lúc chạy lại ra IP khác) — xem `docs/decisions.md` D14.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanTargetKind {
    /// Một địa chỉ IP đơn lẻ, ví dụ `203.0.113.7`.
    Ip,
    /// Một dải CIDR, ví dụ `192.168.10.0/24`.
    Cidr,
}

/// Loại runner test mà M27 chấp nhận (đúng ba loại, không mở rộng).
///
/// Chọn enum **kín** thay vì chuỗi tự do là để `[[qa.suites]]` không thể biến thành đường
/// chạy lệnh tuỳ ý: `argv` luôn do code sinh ra, người khai báo chỉ chọn *runner nào* chứ
/// không chọn *chạy gì* (tinh thần D14.5 của M23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QaRunner {
    /// `cargo test` — dự án Rust.
    CargoTest,
    /// `vitest run` — dự án web (chạy qua `pnpm exec`, không cần `npm`).
    Vitest,
    /// `pytest` — dự án Python.
    Pytest,
}

impl QaRunner {
    /// Runner này có **bắt buộc** cần mạng không? (M27 mục 3 — khoá cứng ở tầng code)
    ///
    /// * `cargo_test` → **có**. `CARGO_HOME=/tmp/cargo` nằm trong container `--rm` nên cache
    ///   crates.io không tồn tại giữa hai lần chạy ⇒ không có mạng thì suite Rust không
    ///   build được. Đây là **ngoại lệ có chủ đích** so với mặc định `--network none` của
    ///   toàn hệ thống, không phải sơ suất (xem `docs/decisions.md` D17.2).
    /// * `vitest`/`pytest` → **không**; dependency nằm sẵn trong image nên chạy offline
    ///   được, và giữ `--network none` là lớp phòng thủ chống test tự gọi mạng.
    ///
    /// Hàm này **cố ý không đọc cấu hình**: `[[qa.suites]]` không có trường nào lật ngược
    /// được cờ mạng. Khi đổi sang image có sẵn toolchain/cache thì đảo quyết định ở
    /// **đúng một chỗ này**.
    #[must_use]
    pub const fn needs_network(self) -> bool {
        matches!(self, Self::CargoTest)
    }
}
