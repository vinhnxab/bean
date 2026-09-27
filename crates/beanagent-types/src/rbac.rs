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
    /// Danh sách trắng tag: role này chỉ thấy tool mang **ít nhất một** tag trong đây.
    ///
    /// **Rỗng ⇒ không áp dụng** (hành vi M21: mọi role đã cấp quyền đều thấy tool untagged).
    /// Cơ chế này sinh ra ở M24 để *tách domain*: role `marketing` phải **không** thấy
    /// `write_file`/`run_shell`/tool `infra-*`, nhưng việc gắn `required_tags` vào các tool
    /// untagged sẽ làm mất chúng cho **mọi** role khác (hồi quy) — nên tách domain phải
    /// làm ở phía *role*, không phải phía tool (D15.1).
    pub allowed_tool_tags: BTreeSet<String>,
}

/// Tag RBAC của domain marketing (M24) — đọc web (dùng chung `web_fetch`/`web_search`).
pub const MARKETING_READ_TAG: &str = "marketing-read";

/// Tag RBAC của domain marketing (M24) — lưu bản nháp, **không** gọi API ngoài.
pub const MARKETING_DRAFT_TAG: &str = "marketing-draft";

/// Tag RBAC của domain marketing (M24) — đăng thật, luôn `Dangerous`.
pub const MARKETING_PUBLISH_TAG: &str = "marketing-publish";

impl RolePermissions {
    /// Quyền **deny-all** — tương đương role `no-access`.
    #[must_use]
    pub fn deny_all(role: &str) -> Self {
        Self {
            role: role.to_string(),
            tags: BTreeSet::new(),
            deny_all: true,
            allowed_tool_tags: BTreeSet::new(),
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
            allowed_tool_tags: BTreeSet::new(),
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
            allowed_tool_tags: BTreeSet::new(),
        }
    }

    /// Dựng quyền **giới hạn theo danh sách trắng tag** (M24).
    ///
    /// Dùng cho vai trò cần *tách domain*: chỉ thấy tool mang tag trong `allowed_tool_tags`,
    /// kể cả khi tool đó untagged. Rỗng ⇒ hành vi M21 (không giới hạn).
    #[must_use]
    pub fn restricted_to(
        role: &str,
        tags: BTreeSet<String>,
        allowed_tool_tags: BTreeSet<String>,
    ) -> Self {
        Self {
            role: role.to_string(),
            tags,
            deny_all: false,
            allowed_tool_tags,
        }
    }

    /// Role này có thấy mọi tool không?
    #[must_use]
    pub fn allows_all(&self) -> bool {
        !self.deny_all && self.tags.contains(WILDCARD_TAG)
    }

    /// **Điểm kiểm tra RBAC duy nhất** (ràng buộc M21.3).
    ///
    /// * `required` — `Tool::required_tags`: tag mà role phải giữ **ít nhất một** để thấy
    ///   tool. Rỗng ⇒ tool untagged, ai đã cấp quyền cũng thấy (hành vi M21).
    /// * `extra` — `Tool::also_visible_to`: tag **bổ sung** cho tool untagged mà vẫn muốn
    ///   một role cụ thể thấy (M24: `web_fetch` cho `marketing`). Mọi tool hiện có đều rỗng
    ///   ⇒ thêm cơ chế này **không** đổi hành vi của bất kỳ cấu hình cũ nào.
    ///
    /// Thứ tự áp dụng: deny-all → toàn quyền → **danh sách trắng tag của role** → tag yêu cầu.
    #[must_use]
    pub fn allows(&self, required: &[&str], extra: &[&str]) -> bool {
        // Bất biến an toàn mặc định: `no-access` không thấy gì, kể cả tool untagged.
        if self.deny_all {
            return false;
        }
        if self.allows_all() {
            return true;
        }
        // (M24) Chế độ tách domain: chỉ tool có tag nằm trong danh sách trắng mới thấy.
        // Tool untagged mà không khai `also_visible_to` sẽ bị ẩn — đúng mục tiêu M24.
        if !self.allowed_tool_tags.is_empty() {
            let permitted = required
                .iter()
                .chain(extra.iter())
                .any(|tag| self.allowed_tool_tags.contains(*tag));
            if !permitted {
                return false;
            }
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

    /// Role này có quyền thấy **báo cáo** của một agent khác không?
    ///
    /// # Vì sao cần một hàm riêng, không tái dùng `allows`
    ///
    /// [`Self::allows`] trả lời "tôi có được **gọi** tool này không" — câu hỏi về
    /// *quyền hành động*. HUB cần câu hỏi khác: "tôi có được **biết** agent này đang
    /// làm gì không". Nguyên tắc chỉ đạt được khi cả hai cùng đúng:
    ///
    /// * `qa` không có `dev-write` nên không được *sửa* code — nhưng nó có
    ///   `dev-read`/`test-run` thuộc **cùng domain** với `developer`, nên nó vẫn thấy
    ///   được công việc đó. Đây đúng là bản chất four-eyes: phải thấy thì mới review.
    /// * `finance-readonly` chỉ giữ `billing-read` — một domain riêng, không giao với
    ///   `dev-write` lẫn `infra-scan`. Nó **không** được biết Developer hay
    ///   Security-scan đang làm gì, đúng như test bắt buộc khẳng định.
    ///
    /// Vì vậy đây là quyết định **thứ hai, tách bạch**: cùng hệ tag RBAC, nhưng trả
    /// lời cho câu hỏi *khả năng quan sát* chứ không phải *quyền hành động*.
    ///
    /// Quy tắc, theo đúng thứ tự (thứ tự này là bất biến bảo mật, không phải tuỳ chọn):
    ///
    /// 1. `deny_all` ⇒ không thấy gì — bất biến fail-closed D11.1.
    /// 2. Tag `*` ⇒ thấy tất cả: đây là quản trị của chính hệ thống.
    /// 3. Nếu có `allowed_tool_tags` (danh sách trắng tách domain M24) thì danh sách
    ///    đó phải giao với tag của agent — kể cả khi tag của người xem có giao.
    /// 4. Còn lại ⇒ thấy khi tập tag của người xem **giao** với tag của agent.
    #[must_use]
    pub fn can_view_agent(&self, agent_tags: &BTreeSet<String>) -> bool {
        if self.deny_all {
            return false;
        }
        if self.tags.contains(WILDCARD_TAG) {
            return true;
        }
        if !self.allowed_tool_tags.is_empty()
            && !self
                .allowed_tool_tags
                .iter()
                .any(|tag| agent_tags.contains(tag))
        {
            return false;
        }
        self.tags.iter().any(|tag| agent_tags.contains(tag))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{RolePermissions, WILDCARD_TAG};

    use std::collections::BTreeSet;

    /// Gọi [`RolePermissions::from_tags`] cho gọn trong test.
    fn from_tags(role: &str, items: &[&str]) -> RolePermissions {
        RolePermissions::from_tags(role, items.iter().map(|s| (*s).to_string()).collect())
    }

    #[test]
    fn deny_all_role_sees_nothing_even_untagged() {
        let perms = RolePermissions::deny_all("no-access");
        assert!(!perms.allows(&[], &[]), "tool untagged phải bị chặn");
        assert!(!perms.allows(&["dev-write"], &[]));
        assert!(!perms.allows_all());
    }

    #[test]
    fn wildcard_sees_everything() {
        let perms = RolePermissions::unrestricted("admin");
        assert!(perms.allows(&[], &[]));
        assert!(perms.allows(&["dev-write"], &[]));
        assert!(perms.allows(&["billing-read"], &[]));
        assert!(perms.allows_all());
    }

    #[test]
    fn untagged_tools_are_visible_to_any_granted_role() {
        let perms = from_tags("qa", &["dev-read", "test-run"]);
        assert!(perms.allows(&[], &[]), "tool chat thường phải dùng được");
    }

    #[test]
    fn tagged_tool_needs_matching_tag() {
        let qa = from_tags("qa", &["dev-read", "test-run"]);
        assert!(qa.allows(&["dev-read"], &[]));
        assert!(
            !qa.allows(&["dev-write"], &[]),
            "four-eyes: qa không được thấy tool dev-write"
        );

        let finance = from_tags("finance-readonly", &["billing-read"]);
        assert!(finance.allows(&["billing-read"], &[]));
        assert!(
            !finance.allows(&["infra-read"], &[]),
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
        assert!(dev.allows(&required, &[]));
        assert!(sec.allows(&required, &[]));
        assert!(!qa.allows(&required, &[]));
    }

    #[test]
    fn usage_scope_is_the_role_name() {
        let perms = from_tags("developer", &["dev-write"]);
        assert_eq!(perms.usage_scope(), "developer");
        assert!(RolePermissions::unrestricted(WILDCARD_TAG).allows(&[], &[]));
    }

    // -----------------------------------------------------------------------
    // `can_view_agent` — quyết định quan sát cho HUB (khác `allows`)
    // -----------------------------------------------------------------------

    fn tags(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn wildcard_role_sees_every_agent() {
        let admin = RolePermissions::unrestricted("admin");
        assert!(admin.can_view_agent(&tags(&["dev-write"])));
        assert!(admin.can_view_agent(&tags(&["billing-read"])));
        assert!(admin.can_view_agent(&tags(&[])));
    }

    #[test]
    fn deny_all_role_sees_no_agent() {
        let none = RolePermissions::deny_all("no-access");
        assert!(!none.can_view_agent(&tags(&["dev-write"])));
        assert!(!none.can_view_agent(&tags(&[])), "rỗng cũng phải bị chặn");
    }

    #[test]
    fn finance_readonly_sees_only_billing_agent() {
        // Đây là test bắt buộc của HUB: role tài chính không được biết trạng thái
        // Developer lẫn Security-scan, và phải thấy chính nó.
        let finance = from_tags("finance-readonly", &["billing-read"]);
        assert!(finance.can_view_agent(&tags(&["billing-read"])));
        assert!(
            !finance.can_view_agent(&tags(&["dev-write"])),
            "tách domain billing khỏi dev"
        );
        assert!(!finance.can_view_agent(&tags(&["infra-read", "infra-scan"])));
        assert!(!finance.can_view_agent(&tags(&["dev-read", "test-run"])));
        assert!(!finance.can_view_agent(&tags(&["marketing-read"])));
    }

    #[test]
    fn review_role_sees_domain_it_reviews_but_cannot_act_on_it() {
        // four-eyes: `qa` không được sửa code (`allows` = false) nhưng PHẢI thấy nó
        // để review. Đây là lý do HUB cần quyết định quan sát tách khỏi quyền hành động.
        let qa = from_tags("qa", &["dev-read", "test-run"]);
        assert!(!qa.allows(&["dev-write"], &[]), "không được gọi tool ghi");
        assert!(
            qa.can_view_agent(&tags(&["dev-read", "test-run"])),
            "phải thấy chính nó"
        );
        assert!(
            !qa.can_view_agent(&tags(&["dev-write"])),
            "Developer là domain khác"
        );
    }

    #[test]
    fn allowed_tool_tags_whitelist_narrows_visibility_further() {
        // `marketing` có cả tag lẫn danh sách trắng: giao với chính nó thì thấy,
        // nhưng bị danh sách trắng chặn với domain khác dù tag có thể trùng.
        let marketing = RolePermissions::restricted_to(
            "marketing",
            tags(&["marketing-read", "marketing-draft"]),
            tags(&["marketing-read", "marketing-draft"]),
        );
        assert!(marketing.can_view_agent(&tags(&["marketing-read"])));
        assert!(!marketing.can_view_agent(&tags(&["dev-write"])));
    }
}
