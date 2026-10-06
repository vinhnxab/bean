//! Section phụ của sản phẩm: `[billing]`, `[scan]`, `[marketing]`, `[qa]`.

use serde::{Deserialize, Serialize};

use super::*;

/// `[billing]` — domain tài chính, read-only (M22a).
///
/// Tách biệt hoàn toàn khỏi domain `infra-*`: tool ở đây chỉ **đọc chi phí**, không đụng
/// tới hạ tầng.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BillingConfig {
    /// Bật tool đọc chi phí. Mặc định `false`.
    pub enabled: bool,
    /// Tên biến môi trường chứa credential billing.
    ///
    /// **Phải là biến riêng** — `validate()` từ chối nếu trùng với biến của LLM, web search
    /// hay Telegram, vì credential có quyền quản trị hạ tầng không được dùng lại ở đây
    /// (`Plan.md` mục 18, D13.2).
    pub api_key_env: String,
    /// Endpoint trả JSON chi phí. `None` ⇒ tool chạy ở **chế độ stub**, không gọi mạng.
    pub base_url: Option<String>,
    /// Gắn thêm tham số truy vấn vào `base_url` khi gọi (ví dụ `?period=30d`).
    pub query_suffix: Option<String>,
}

impl Default for BillingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key_env: "CLOUD_BILLING_API_KEY".to_string(),
            base_url: None,
            query_suffix: None,
        }
    }
}

/// Một mục `[[infra_scope]]` — target được phép quét (M23).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScanScopeEntry {
    /// Kiểu target.
    pub kind: ScanTargetKind,
    /// Giá trị: IP hoặc CIDR. Cú pháp được `validate()` kiểm tra nghiêm ngặt.
    pub value: String,
    /// Nhãn mô tả để người đọc báo cáo hiểu đang quét cái gì (tùy chọn).
    #[serde(default)]
    pub label: String,
}

/// Sandbox riêng cho tool quét (M23).
///
/// **Tách khỏi** `[security.sandbox]` của `run_shell` một cách tuyệt đối: scanner **bắt buộc**
/// phải có mạng để tới target, trong khi `run_shell` phải `--network none`. Dùng chung cấu
/// hình sẽ vô hiệu hoá cách ly của `run_shell` (D14.3).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanSandboxConfig {
    /// `docker` (mặc định, an toàn) hoặc `host` (chạy thẳng trên máy — phải bật tường minh).
    pub mode: SandboxMode,
    /// Image chứa scanner (chỉ dùng khi `mode = "docker"`).
    pub image: String,
    /// Cho container scanner ra mạng. **Luôn phải là `true`** — không có mạng thì không quét
    /// được target trong scope; `validate()` từ chối `false`.
    pub network: bool,
    /// Trần bộ nhớ.
    pub memory: String,
    /// Số CPU.
    pub cpus: f32,
    /// Trần số tiến trình trong container (chống fork bomb).
    pub pids_limit: u32,
    /// Thời gian tối đa cho một lần quét (giây).
    pub timeout_seconds: u64,
    /// Cho phép chạy scanner thẳng trên máy chủ. Mặc định `false`.
    ///
    /// `mode = "host"` mà quên bật cờ này sẽ bị `validate()` từ chối — chốt chặn chống vô
    /// tình hạ cấp cách ly (D14.4).
    #[serde(default)]
    pub allow_host: bool,
}

impl Default for ScanSandboxConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::Docker,
            image: "instrumentisto/nmap:latest".to_string(),
            network: true,
            memory: "512m".to_string(),
            cpus: 1.0,
            pids_limit: 64,
            timeout_seconds: 300,
            allow_host: false,
        }
    }
}

impl ScanSandboxConfig {
    /// Thành [`SandboxConfig`] để dựng `Sandbox`, giữ nguyên mọi giới hạn an toàn sẵn có
    /// (non-root, `--cap-drop ALL`, no-new-privileges, trần bộ nhớ/CPU/pids).
    ///
    /// `SandboxConfig` không mang workspace — workspace truyền riêng khi dựng `Sandbox`.
    #[must_use]
    pub fn to_sandbox_config(&self) -> SandboxConfig {
        SandboxConfig {
            mode: self.mode,
            image: self.image.clone(),
            network: self.network,
            memory: self.memory.clone(),
            cpus: self.cpus,
            pids_limit: self.pids_limit,
            timeout_seconds: self.timeout_seconds,
        }
    }
}

/// Cấu hình domain marketing (M24).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarketingConfig {
    /// Bật nhóm tool marketing.
    pub enabled: bool,
    /// Tên biến môi trường chứa API key **riêng** của nền tảng publish.
    ///
    /// `validate()` từ chối trùng với `llm.api_key_env` / `tools.web_search.api_key_env` /
    /// `telegram.token_env` — cùng mẫu với M22a: quyền "chỉ post" phải là credential riêng,
    /// không dùng lại key có quyền rộng hơn.
    pub api_key_env: String,
    /// Endpoint nhận nội dung để đăng (JSON). Rỗng ⇒ **chế độ stub**: không gọi mạng.
    pub base_url: Option<String>,
    /// Trường JSON chứa nội dung trong body, ví dụ `"text"`.
    pub text_field: String,
}

impl Default for MarketingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key_env: "MARKETING_POST_ONLY_KEY".to_string(),
            base_url: None,
            // `text` là tên trường phổ biến nhất; vẫn cấu hình được nếu provider khác.
            text_field: "text".to_string(),
        }
    }
}

/// Một test suite khai trong `[[qa.suites]]` (M27).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QaSuiteConfig {
    /// Tên suite — model chỉ được chọn đúng giá trị này.
    pub name: String,
    /// Loại runner (xem [`QaRunner`]).
    pub runner: QaRunner,
    /// Thư mục chạy, **tương đối** so với workspace (ví dụ `web`). Rỗng = gốc workspace.
    ///
    /// Không nhận đường dẫn tuyệt đối hay `..` — `validate_qa()` chặn, và
    /// `Sandbox::run_argv_readonly` chặn lặp ở tầng cuối.
    #[serde(default)]
    pub workdir: String,
    /// Tham số **cố định** thêm vào trước (ví dụ `["--workspace"]` cho cargo).
    ///
    /// Đây là cấu hình của *người quản trị*, không phải của model: model chỉ chọn `suite`
    /// và `filter`. `validate_qa()` chặn tham số rỗng.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Sandbox riêng cho `qa_test` (M27).
///
/// **Tách khỏi** `[security.sandbox]` của `run_shell` (đúng tinh thần D14.3 của M23): runner
/// test cần toolchain, thư mục làm việc và quyền ghi *bên trong container* (target/, cache)
/// trong khi workspace của bạn phải mount **read-only**. Dùng chung cấu hình sẽ phá cả
/// hai hướng bảo vệ.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct QaSandboxConfig {
    /// Image chứa toolchain của runner (Rust/Node/Python tương ứng từng suite).
    pub image: String,
    /// Cho container ra mạng hay không.
    ///
    /// `validate_qa()` **ghi đè giá trị này** theo từng suite từ [`QaRunner::needs_network`]
    /// và cảnh báo nếu người khai báo khác — cờ mạng không phải thứ cấu hình tự do.
    pub network: bool,
    /// Trần bộ nhớ.
    pub memory: String,
    /// Số CPU.
    pub cpus: f32,
    /// Trần số tiến trình trong container (chống fork bomb).
    pub pids_limit: u32,
    /// Thời gian tối đa cho một lần chạy suite (giây). Test thường lâu hơn shell nhiều.
    pub timeout_seconds: u64,
}

impl Default for QaSandboxConfig {
    fn default() -> Self {
        Self {
            image: "Bean-sandbox:latest".to_string(),
            // Mặc định an toàn; `validate_qa()` bật lại cho `cargo_test` (D17.2).
            network: false,
            memory: "2g".to_string(),
            cpus: 2.0,
            pids_limit: 256,
            timeout_seconds: 900,
        }
    }
}

impl QaSandboxConfig {
    /// Thành [`SandboxConfig`] để dựng `Sandbox`, giữ nguyên mọi giới hạn an toàn sẵn có
    /// (non-root, `--cap-drop ALL`, no-new-privileges, trần bộ nhớ/CPU/pids).
    ///
    /// `mode` **cố ý không lấy từ cấu hình** và luôn là `Docker`: `qa_test` dựa vào mount
    /// `:ro` để giữ four-eyes, mà ở chế độ host không thể bảo đảm được điều đó
    /// (`run_argv_readonly` từ chối chế độ host). Nhờ vậy cấu hình không có trường `mode`
    /// ⇒ không tồn tại cách nào lỡ tay hạ cấp cách ly, khác D14.4 của M23 vốn cho phép
    /// host kèm cờ `allow_host` tường minh.
    #[must_use]
    pub fn to_sandbox_config(&self) -> SandboxConfig {
        SandboxConfig {
            mode: SandboxMode::Docker,
            image: self.image.clone(),
            network: self.network,
            memory: self.memory.clone(),
            cpus: self.cpus,
            pids_limit: self.pids_limit,
            timeout_seconds: self.timeout_seconds,
        }
    }
}

/// `[qa]` — domain chạy test suite cho vai trò `qa` (M27).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct QaConfig {
    /// Bật nhóm tool QA (còn phải có `[[qa.suites]]` không rỗng mới chạy được).
    pub enabled: bool,
    /// Sandbox riêng cho runner test.
    pub sandbox: QaSandboxConfig,
    /// Danh sách suite được phép chạy. **Rỗng ⇒ mọi lần gọi đều bị từ chối** (fail-closed,
    /// đúng như `[[infra_scope]]` rỗng của M23).
    pub suites: Vec<QaSuiteConfig>,
}

/// Cấu hình domain `security-scan` (M23).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityScanConfig {
    /// Bật nhóm tool quét.
    pub enabled: bool,
    /// Sandbox riêng cho scanner.
    pub sandbox: ScanSandboxConfig,
    /// Kênh nhận cảnh báo mức cao (thường là kênh chính của bạn).
    ///
    /// Rỗng ⇒ không gửi cảnh báo trực tiếp. `Plan.md` yêu cầu cảnh báo mức cao gửi **thẳng**
    /// cho bạn, song song với báo cáo chuẩn hoá gửi Manager — không chỉ đi qua Manager lọc.
    pub alert_channel: String,
    /// `chat_id` của kênh cảnh báo.
    pub alert_chat_id: String,
}
