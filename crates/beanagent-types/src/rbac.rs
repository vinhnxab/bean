//! RBAC: quyền của một role đã resolve, tuần tự hoá được (Plan.md M21, mục 4.1/4.3).
//!
//! # Vì sao một kiểu riêng
//!
//! `Plan.md` mục 4 ràng buộc **ba** điều cho mọi milestone:
//!
//! 1. Manager ↔ agent con giao tiếp qua kiểu **tuần tự hoá được**, không truyền tham chiếu
//!    Rust nội bộ (`&mut`, `Arc<RwLock<…>>`).
//! 2. Mọi truy cập SQLite đi qua đúng một lớp worker.
//! 3. **RBAC check nằm ở đúng một điểm** trong Router, không rải rác trong logic từng tool.
//!
//! `RolePermissions` là hiện thân của cả (1) và (3): Router **quyết định một lần** rồi truyền
//! struct này (derive `Serialize`/`Deserialize`) xuống agent loop. Agent loop **không** tự tra
//! cứu role, chỉ *áp dụng* kết quả đã quyết định. Nhờ vậy không có logic RBAC nào nằm rải trong
//! tool/role, và mọi thao tác kiểm tra đều đi qua **một** hàm [`RolePermissions::allows`].
//!
//! # Ngữ nghĩa thẻ (tag)
//!
//! * Tool khai báo `required_tags = &[]` ⇒ *"không cần thẻ đặc biệt"*.
//! * Role có tag `"*"` ([`WILDCARD_TAG`]) ⇒ thấy **mọi** tool.
//! * Ngược lại, tool có `required_tags` khác rỗng chỉ hiện khi role giữ **ít nhất một** tag
//!   trong `required_tags` (ngữ nghĩa OR) — đủ để `run_shell` chạy được cho cả `dev-write`
//!   lẫn `infra-scan` mà vẫn chặn `qa`.
//! * **`no-access` là deny-all** ([`NO_ACCESS_ROLE`]): không thấy tool nào, kể cả untagged.
//!   Đây là bất biến an toàn mặc định (Plan.md mục 2), **không** được đổi thành `admin`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Tag đặc biệt: role này thấy **mọi** tool, kể cả tool tagged.
pub const WILDCARD_TAG: &str = "*";

/// Tên role mặc định cho user không có trong `agent.user_roles`.
///
/// Role **deny-all**: không thấy tool nào, kể cả tool `required_tags = &[]` (D10.2).
/// An toàn theo mặc định — tuyệt đối không đổi thành `admin`.
pub const NO_ACCESS_ROLE: &str = "no-access";

/// Alias đọc rõ ở nơi gọi (`allows_all`).
pub const ALL_TAGS: &str = WILDCARD_TAG;

/// Quyền của một role **đã resolve** cho một user cụ thể.
///
/// Chuỗi tuần tự hoá được ⇒ có thể truyền qua ranh giới tiến trình sau này (ràng buộc M21.1)
/// mà không phải đổi thiết kế.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolePermissions {
    /// Tên role sau khi resolve (dùng cho log/audit và làm khoá ngân sách token theo role).
    pub role: String,
    /// Tập tag mà role giữ. Có chứa [`WILDCARD_TAG`] nghĩa là thấy mọi tool.
    pub tags: BTreeSet<String>,
    /// `true` ⇒ deny-all: không thấy tool nào, kể cả untagged (role `no-access`).
    pub deny_all: bool,
}

impl RolePermissions {
    /// Quyền **deny-all** — tương đương role `no-access`.
    #[must_use]
    pub fn deny_all(role: &str) -> Self {
        Self {
            role: role.to_string(),
            tags: BTreeSet::new(),
            deny_all: true,
        }
    }

    /// Quyền toàn quyền (tag `*`) — dùng khi RBAC **không** bật (giữ hành vi cũ một-người-dùng).
    #[must_use]
    pub fn unrestricted(role: &str) -> Self {
        let mut tags = BTreeSet::new();
        tags.insert(WILDCARD_TAG.to_string());
        Self {
            role: role.to_string(),
            tags,
            deny_all: false,
        }
    }

    /// Dựng quyền từ tập tag.
    ///
    /// Một role khai báo trong cấu hình mà **không** có tag nào vẫn dùng được tool untagged
    /// (khác `no-access`, vốn deny-all theo bất biến) — nên `deny_all` chỉ do
    /// [`Self::deny_all`] đặt tường minh.
    #[must_use]
    pub fn from_tags(role: &str, tags: BTreeSet<String>) -> Self {
        Self {
            role: role.to_string(),
            tags,
            deny_all: false,
        }
    }

    /// Role này có thấy mọi tool không?
    #[must_use]
    pub fn allows_all(&self) -> bool {
        !self.deny_all && self.tags.contains(WILDCARD_TAG)
    }

    /// **Điểm kiểm tra RBAC duy nhất** (ràng buộc M21.3).
    ///
    /// `required` là `Tool::required_tags` của tool; rỗng nghĩa là tool không nhạy cảm.
    #[must_use]
    pub fn allows(&self, required: &[&str]) -> bool {
        // Bất biến an toàn mặc định: `no-access` không thấy gì, kể cả tool untagged.
        if self.deny_all {
            return false;
        }
        if self.allows_all() {
            return true;
        }
        if required.is_empty() {
            return true;
        }
        required
            .iter()
            .any(|tag| self.tags.contains(*tag) || self.tags.contains(WILDCARD_TAG))
    }

    /// Khoá tính ngân sách token theo role (ràng buộc M21.7).
    ///
    /// Ngân sách **tách riêng theo từng role** để Developer chạy vòng lặp dài không ăn hết
    /// hạn mức khiến Monitor/Security-scan không chạy được job định kỳ.
    #[must_use]
    pub fn usage_scope(&self) -> &str {
        &self.role
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{RolePermissions, WILDCARD_TAG};

    /// Gọi [`RolePermissions::from_tags`] cho gọn trong test.
    fn from_tags(role: &str, items: &[&str]) -> RolePermissions {
        RolePermissions::from_tags(role, items.iter().map(|s| (*s).to_string()).collect())
    }

    #[test]
    fn deny_all_role_sees_nothing_even_untagged() {
        let perms = RolePermissions::deny_all("no-access");
        assert!(!perms.allows(&[]), "tool untagged phải bị chặn");
        assert!(!perms.allows(&["dev-write"]));
        assert!(!perms.allows_all());
    }

    #[test]
    fn wildcard_sees_everything() {
        let perms = RolePermissions::unrestricted("admin");
        assert!(perms.allows(&[]));
        assert!(perms.allows(&["dev-write"]));
        assert!(perms.allows(&["billing-read"]));
        assert!(perms.allows_all());
    }

    #[test]
    fn untagged_tools_are_visible_to_any_granted_role() {
        let perms = from_tags("qa", &["dev-read", "test-run"]);
        assert!(perms.allows(&[]), "tool chat thường phải dùng được");
    }

    #[test]
    fn tagged_tool_needs_matching_tag() {
        let qa = from_tags("qa", &["dev-read", "test-run"]);
        assert!(qa.allows(&["dev-read"]));
        assert!(
            !qa.allows(&["dev-write"]),
            "four-eyes: qa không được thấy tool dev-write"
        );

        let finance = from_tags("finance-readonly", &["billing-read"]);
        assert!(finance.allows(&["billing-read"]));
        assert!(
            !finance.allows(&["infra-read"]),
            "tách domain billing khỏi infra"
        );
    }

    #[test]
    fn required_tags_use_or_semantics() {
        // `run_shell` mang cả `dev-write` lẫn `infra-scan`: đủ một tag là được gọi.
        let required = ["dev-write", "infra-scan"];
        let dev = from_tags("developer", &["dev-write"]);
        let sec = from_tags("security-scan", &["infra-scan"]);
        let qa = from_tags("qa", &["dev-read", "test-run"]);
        assert!(dev.allows(&required));
        assert!(sec.allows(&required));
        assert!(!qa.allows(&required));
    }

    #[test]
    fn usage_scope_is_the_role_name() {
        let perms = from_tags("developer", &["dev-write"]);
        assert_eq!(perms.usage_scope(), "developer");
        assert!(RolePermissions::unrestricted(WILDCARD_TAG).allows(&[]));
    }
}
