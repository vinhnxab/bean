//! Section `[agent]`, `[roles]`, `[projects]`, `[data]`.

use std::collections::{BTreeMap, BTreeSet};

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::router::AgentRelation;

use super::*;

/// `[agent]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// Thư mục làm việc của agent (mọi thao tác file bị jail trong đây).
    pub workspace: PathBuf,
    /// Tên agent hiển thị trong prompt và log.
    pub agent_name: String,
    /// Trần số bước của một run (agents.md mục 6).
    pub max_steps: u32,
    /// Ngân sách token cho context gửi model (mục 8.2, 8.3).
    pub context_budget_tokens: u32,
    /// Múi giờ IANA dùng để parse cron và hiển thị phía server (mục 14, D5.8).
    pub timezone: String,
    /// Lớp kiểm tra thứ hai ở lõi, ngoài allowlist của từng kênh (mục 10).
    pub allowed_users: Vec<String>,
    /// Map `user_id` (`telegram:<id>`, `web:admin`, `cli:local`) → tên role (M21.3).
    ///
    /// User **không** có trong map này dùng role mặc định
    /// [`NO_ACCESS_ROLE`](crate::NO_ACCESS_ROLE) — deny-all, an toàn theo mặc định.
    ///
    /// RBAC chỉ **bật** khi map này khác rỗng; để trống thì mọi user trong `allowed_users`
    /// giữ hành vi cũ (thấy mọi tool) để không phá cài đặt một-người-dùng (D10.3).
    pub user_roles: BTreeMap<String, String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("./workspace"),
            agent_name: "Bean".to_string(),
            max_steps: 25,
            context_budget_tokens: 100_000,
            timezone: "Asia/Ho_Chi_Minh".to_string(),
            allowed_users: vec!["web:admin".to_string(), "cli:local".to_string()],
            user_roles: BTreeMap::new(),
        }
    }
}

/// Một role trong bảng `[[roles]]` (M21.2).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoleConfig {
    /// Tên role, khớp giá trị trong `agent.user_roles`.
    pub name: String,
    /// Các tag mà role được cấp. Tag `"*"` nghĩa là **mọi** quyền.
    #[serde(default)]
    pub tool_tags: Vec<String>,
    /// Tag **cấm** cứng cho role này; config sai thì `validate()` báo lỗi (M21.6).
    ///
    /// Dùng cho nguyên tắc four-eyes: role `qa` khai báo
    /// `forbid_tags = ["dev-write"]` nên **không thể** lỡ tay cấp quyền ghi code cho role
    /// review — kiểm tra ở tầng code, không dựa vào model tự kiểm tra.
    #[serde(default)]
    pub forbid_tags: Vec<String>,
    /// **Danh sách trắng tag**: role này chỉ thấy tool mang ít nhất một tag trong đây.
    ///
    /// Mặc định **rỗng ⇒ không áp dụng** (mọi role đã cấp quyền đều thấy tool untagged) —
    /// nên thêm trường này không đổi hành vi của cấu hình cũ nào.
    ///
    /// Cơ chế này sinh ra ở M24 để **tách domain**: role `marketing` phải thấy
    /// `web_fetch`/`web_search` mà **không** thấy `write_file`/`run_shell`/tool `infra-*`.
    /// Không thể làm bằng `tool_tags` vì gắn tag vào một tool untagged sẽ *giấu nó khỏi mọi
    /// role khác* — hồi quy cho các cài đặt đang chạy (D15.1).
    #[serde(default)]
    pub allowed_tool_tags: Vec<String>,
    /// Ghi đè `agent.context_budget_tokens` cho riêng role này (M21.7).
    #[serde(default)]
    pub context_budget_tokens: Option<u32>,
    /// Ghi đè `security.daily_token_budget` cho riêng role này (M21.7).
    #[serde(default)]
    pub daily_token_budget: Option<u64>,
}

impl RoleConfig {
    /// Tập tag đã chuẩn hoá (loại khoảng trắng thừa, bỏ tag rỗng).
    #[must_use]
    pub fn tag_set(&self) -> BTreeSet<String> {
        self.tool_tags
            .iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect()
    }

    /// Tập `allowed_tool_tags` đã chuẩn hoá (danh sách trắng tách domain — M24).
    #[must_use]
    pub fn allowed_tag_set(&self) -> BTreeSet<String> {
        self.allowed_tool_tags
            .iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect()
    }

    /// Quan hệ kiến trúc của role này với Manager, **suy ra từ tag được cấp**.
    ///
    /// # Vì sao suy ra từ tag chứ không khớp tên role
    ///
    /// Tên role (`developer`, `qa`, …) là lựa chọn của người cấu hình; cấu hình
    /// hoàn toàn hợp lệ có thể đặt tên khác (`dev`, `reviewer`). Nếu UI hardcode
    /// theo tên, một bản triển khai đổi tên sẽ **vẽ sai kiến trúc** mà không có
    /// ai báo lỗi — kiểu hỏng âm thầm nguy hiểm nhất.
    ///
    /// Tag thì ngược lại: nó là hợp đồng RBAC đã được kiểm ở tầng code
    /// (`validate_rbac`), và sửa đổi tên role không đổi được hành vi. Suy ra từ
    /// tag nên sơ đồ luôn mô tả đúng hệ thống đang chạy.
    ///
    /// Thứ tự ưu tiên có chủ đích:
    ///
    /// 1. `infra-scan` → [`AgentRelation::AlertsDirectly`]. Đây là kênh D14.11:
    ///    cảnh báo mức cao đi thẳng tới người quản trị, **không qua Manager**.
    ///    Kiểm trước vì vai trò trực trật thường *cũng* đọc hạ tầng.
    /// 2. Có tag đọc/kiểm thử nhưng **không** có tag ghi → [`AgentRelation::Reviews`].
    ///    Đây chính là định nghĩa four-eyes ở tầng dữ liệu: agent review được
    ///    đọc và chạy test nhưng không có quyền sửa, nên nó không thể tự duyệt
    ///    thứ nó đang review. `qa` rơi vào nhánh này vì `validate_rbac` đã chặn
    ///    việc nó giữ `dev-write`.
    /// 3. Còn lại → [`AgentRelation::Manages`].
    ///
    /// [`AgentRelation`]: crate::router::AgentRelation
    #[must_use]
    pub fn relation(&self) -> AgentRelation {
        let tags = self.tag_set();
        if tags.contains(INFRA_SCAN_TAG) {
            return AgentRelation::AlertsDirectly;
        }
        let can_write = tags.contains(DEV_WRITE_TAG);
        let inspects = tags.contains(DEV_READ_TAG) || tags.contains(TEST_RUN_TAG);
        if !can_write && inspects {
            return AgentRelation::Reviews;
        }
        AgentRelation::Manages
    }
}

/// Một project profile trong `[[projects]]` (M21.1).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    /// Tên project (kebab-case), dùng làm khoá chọn project khi mở phiên.
    pub name: String,
    /// Workspace riêng của project; mọi thao tác file của project bị jail trong đây.
    pub workspace: PathBuf,
}

/// `[data]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DataConfig {
    /// Nơi chứa SQLite, audit log, `auth.toml`.
    pub dir: PathBuf,
}

impl Default for DataConfig {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("~/.bean"),
        }
    }
}
