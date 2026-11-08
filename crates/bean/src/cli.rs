//! Định nghĩa CLI bằng `clap` (agents.md mục 18: `chat`, `serve`, `auth`).

use std::path::{Path, PathBuf};

use bean_types::config::DEFAULT_CONFIG_FILE;
use clap::{Args, Parser, Subcommand};

/// Trợ lý AI cá nhân tự host: một binary Rust (CLI + web + Telegram).
#[derive(Debug, Parser)]
#[command(name = "bean", version, about, long_about = None)]
pub struct Cli {
    /// File cấu hình TOML. Mặc định dùng `./bean.toml` nếu file này tồn tại.
    #[arg(long, global = true, env = "BEAN_CONFIG", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Lệnh cần chạy.
    #[command(subcommand)]
    pub command: Command,
}

/// Các lệnh con.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// REPL trên terminal (không cần web, không cần Node).
    Chat(ChatArgs),

    /// Chạy web + Telegram trong một tiến trình.
    Serve(ServeArgs),

    /// Tiện ích xác thực cho giao diện web.
    Auth(AuthArgs),

    /// Chạy Bean ở chế độ **MCP server read-only** (M25) cho Cline/Cursor/OpenCode…
    Mcp(McpArgs),
    Secret(SecretCommand),
    Config(ConfigCommand),

    /// Kiểm tra sức khoẻ cấu hình: quyền file, key sai chỗ, slot bị env che, proxy
    /// (K29), tiến trình/port (K27), sandbox Docker, Chrome snap — không bao giờ in
    /// giá trị secret. Mã thoát: 0 ok, 1 lỗi, 2 cảnh báo.
    Doctor {
        /// Thử request tối thiểu tới LLM/voice/Telegram/web search (tốn một call nhỏ,
        /// opt-in).
        #[arg(long)]
        online: bool,
    },

    /// Cài đặt tương tác ban đầu (idempotent): nhà cung cấp + key, mật khẩu web,
    /// Telegram, tìm kiếm web, giọng nói; mỗi bước có thể bỏ qua.
    Setup,

    /// Chọn nhà cung cấp/mô hình tương tác, lưu vào `llm-state.json` (cùng nơi
    /// với giao diện web) và áp dụng khi khởi động lại Bean.
    Model,

    /// Kiểm tra và cài bản mới từ GitHub Releases (M30): tải/cài bản mới đúng nền
    /// tảng, xác minh SHA256, ghi đè binary an toàn rồi tự restart qua
    /// `scripts/beanctl` (D23). Chỉ nhận bản stable; check tay, không phone-home.
    Update {
        /// Chỉ kiểm tra có bản mới, không tải hay cài.
        #[arg(long)]
        check: bool,

        /// Sau khi cài xong KHÔNG tự restart Bean.
        #[arg(long)]
        no_restart: bool,
    },

    /// Chạy bộ eval đo chất lượng agent (crate `bean-eval`): task TOML trong
    /// `evals/tasks/`, ghi metrics JSONL + trajectory để so sánh trước/sau khi
    /// tối ưu. Mọi run nằm trong thư mục tạm — không đụng dữ liệu thật.
    Eval(EvalArgs),
}

/// Tham số của `eval`.
#[derive(Debug, Args)]
pub struct EvalArgs {
    /// Lệnh con của `eval`.
    #[command(subcommand)]
    pub command: EvalCommand,
}

/// Lệnh con của `eval`.
#[derive(Debug, Subcommand)]
pub enum EvalCommand {
    /// Chạy các task eval và ghi metrics + trajectory.
    Run {
        /// Thư mục chứa task TOML.
        #[arg(long, value_name = "DIR", default_value = "evals/tasks")]
        tasks: PathBuf,

        /// Chỉ chạy task có id này (lặp được).
        #[arg(long, value_name = "ID")]
        only: Option<String>,

        /// fake (kịch bản) | real (provider thật, key từ env) — mặc định tự theo task.
        #[arg(long, value_name = "MODE")]
        provider: Option<String>,

        /// Thư mục ghi metrics.jsonl + trajectory/.
        #[arg(long, value_name = "DIR", default_value = "evals/out")]
        out: PathBuf,
    },

    /// Đọc file metrics JSONL và in báo cáo tổng hợp ra stdout.
    Report {
        /// File metrics JSONL.
        file: PathBuf,
    },
}

/// Tham số của `mcp`.
#[derive(Debug, Args)]
pub struct McpArgs {
    /// Lệnh con của `mcp`.
    #[command(subcommand)]
    pub command: McpCommand,
}

/// Các lệnh con của `mcp`.
#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Chạy MCP server. Mặc định `stdio` (Bean cùng máy với IDE);
    /// `--http` dùng streamable-HTTP/SSE (Bean chạy remote, sau reverse proxy).
    Serve {
        /// Dùng transport streamable-HTTP thay vì stdio.
        #[arg(long)]
        http: bool,
    },
}

/// Tham số của `chat`.
#[derive(Debug, Args)]
pub struct ChatArgs {
    /// Dùng kịch bản JSON làm provider giả thay vì gọi API thật.
    #[arg(long, value_name = "FILE")]
    pub fake_llm: Option<PathBuf>,

    /// Ghi đè thư mục workspace (`[agent] workspace`) — tiện cho demo/test.
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
}

/// Tham số của `serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Dùng kịch bản JSON làm provider giả (dùng cho test end-to-end).
    #[arg(long, value_name = "FILE")]
    pub fake_llm: Option<PathBuf>,
}

/// Tham số của `auth`.
#[derive(Debug, Args)]
pub struct AuthArgs {
    /// Lệnh con của `auth`.
    #[command(subcommand)]
    pub command: AuthCommand,
}

/// Các lệnh con của `auth`.
#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Đặt mật khẩu cho giao diện web (lưu hash argon2id vào `data.dir/auth.toml`).
    SetPassword,

    /// Quản lý token của client MCP (M25): `add` / `list` / `revoke`.
    McpToken(McpTokenArgs),
}

/// Tham số của `auth mcp-token`.
#[derive(Debug, Args)]
pub struct McpTokenArgs {
    /// Lệnh con của `mcp-token`.
    #[command(subcommand)]
    pub command: McpTokenCommand,
}

/// Quản lý kho secret thống nhất `data.dir/secrets.env` (M29).
#[derive(Debug, Args)]
pub struct SecretCommand {
    #[command(subcommand)]
    pub command: SecretCommandAction,
}

#[derive(Debug, Subcommand)]
pub enum SecretCommandAction {
    /// Nhập ẩn API key vào kho secret.
    Set {
        /// Tên slot secret (ví dụ ANTHROPIC_API_KEY).
        name: String,
        /// Đọc giá trị từ stdin thay vì nhập ẩn từ terminal.
        #[arg(long)]
        stdin: bool,
    },
    /// Liệt kê các slot secret và trạng thái.
    List,
    /// Xoá một secret khỏi kho.
    Unset {
        /// Tên slot secret.
        name: String,
    },
}

/// Quản lý bảng thiết lập thống nhất (M29).
#[derive(Debug, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: ConfigCommandAction,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommandAction {
    /// Di chuyển key mặc định từ bean.toml vào secrets.env.
    MigrateSecrets {
        /// Áp dụng thay đổi ngay (mặc định chỉ xem trước).
        #[arg(long)]
        apply: bool,
        /// Đọc key từ file ~/.config/bean/env thay vì nhập tay.
        #[arg(long)]
        from_env_file: Option<PathBuf>,
    },
    /// Liệt kê bảng thiết lập.
    List,
    /// Xem giá trị một khoá.
    Get {
        /// Đường dẫn khoá (ví dụ agent.timezone).
        key: String,
    },
    /// Ghi giá trị một khoá (chỉ tầng A).
    Set {
        /// Đường dẫn khoá.
        key: String,
        /// Giá trị mới.
        value: String,
    },
    /// Xoá ghi đè một khoá (về mặc định).
    Unset {
        /// Đường dẫn khoá.
        key: String,
    },
}

/// Các lệnh con của `auth mcp-token`.
#[derive(Debug, Subcommand)]
pub enum McpTokenCommand {
    /// Cấp token mới cho một client (in token thô **một lần duy nhất**).
    Add {
        /// Tên client, phải có trong `[[mcp_clients]]`.
        name: String,
    },
    /// Liệt kê client đã được cấp token.
    List,
    /// Thu hồi token của một client.
    Revoke {
        /// Tên client cần thu hồi.
        name: String,
    },
}

impl Cli {
    /// File cấu hình hiệu dụng: `--config`, hoặc `./bean.toml` nếu tồn tại.
    #[must_use]
    pub fn config_path(&self) -> Option<&Path> {
        if let Some(path) = self.config.as_deref() {
            return Some(path);
        }
        let default = Path::new(DEFAULT_CONFIG_FILE);
        if default.is_file() {
            Some(default)
        } else {
            None
        }
    }
}
