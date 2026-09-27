//! Xác thực client MCP (M25) — token dài hạn, chỉ lưu **hash**.
//!
//! # Vì sao tách khỏi cookie phiên web
//!
//! `AuthService` sinh token **phiên** (TTL ngắn, gắn cookie, đổi mỗi lần đăng nhập).
//! Token MCP thì **dài hạn** và phải đặt trong file cấu hình của Cline/Cursor — tức là
//! nằm trong thư mục dự án, thường được commit. Nên:
//!
//! * Bean **không** tái dùng `web_sessions`: thu hồi token MCP không được xoá phiên
//!   đăng nhập của bạn và ngược lại.
//! * Token thô chỉ in **một lần** lúc `auth mcp-token add`; Bean lưu SHA-256 trong bảng
//!   `mcp_clients` (`data.dir/beanagent.db`, không nằm trong git).
//!
//! # Tra ở đâu
//!
//! [`McpAuth::authenticate`] được gọi ở **bước handshake** (`initialize`), trước khi
//! client thấy bất kỳ tool nào. Sai ⇒ lỗi protocol, client không vào được phiên.

use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

use crate::store::{Store, StoreError};

/// Số byte token MCP (256 bit) — cùng cấp với token phiên web.
pub const MCP_TOKEN_BYTES: usize = 32;

/// Hash SHA-256 của token, dạng hex 64 ký tự (thứ tự byte giống `beanagent_web::auth`).
#[must_use]
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        // Ghi vào `String` đã `with_capacity` không thể thất bại; `_ =` để không vi phạm
        // quy tắc "không unwrap" của workspace.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Sinh token ngẫu nhiên 256 bit dạng hex.
///
/// # Errors
/// Trả `Err` nếu PRNG của hệ điều hành không cấp được entropy — thay vì sinh token yếu.
pub fn new_token() -> Result<String, String> {
    let mut bytes = [0_u8; MCP_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| format!("không sinh được token: {error}"))?;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

/// Danh tính của một client MCP đã xác thực + quyền đã resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpClientIdentity {
    /// Tên client (không kèm tiền tố), ví dụ `cline`.
    pub name: String,
    /// Identity đầy đủ `mcp-client:<name>` — đây là khoá tra RBAC.
    pub user_id: String,
    /// Tên role sau khi resolve (dùng cho audit và ngân sách token).
    pub role: String,
}

/// Lỗi xác thực client MCP.
#[derive(Debug, thiserror::Error)]
pub enum McpAuthError {
    /// Token thiếu, sai định dạng, không khớp bản ghi nào, hoặc đã hết hạn.
    ///
    /// Cố tình **không** phân biệt các nguyên nhân trong thông điệp trả về cho client
    /// (đừng biến thành oracle dò tìm token).
    #[error("token không hợp lệ hoặc đã hết hạn")]
    InvalidToken,
    /// Token hợp lệ nhưng client không còn trong `[[mcp_clients]]` của cấu hình.
    ///
    /// Tách khỏi [`Self::InvalidToken`] vì đây là lỗi **cấu hình**, cần log rõ để
    /// người dùng biết phải sửa `BeanAgent.toml` chứ không phải cấp lại token.
    #[error("client `{0}` không có trong [[mcp_clients]] — thêm entry trong BeanAgent.toml")]
    ClientNotConfigured(String),
    /// Lỗi store khi tra token.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Bộ xác thực token MCP: băm token rồi tra trong bảng `mcp_clients`.
pub struct McpAuth {
    store: std::sync::Arc<dyn Store>,
}

impl McpAuth {
    /// Dựng từ store (đi qua **một** worker SQLite duy nhất — ràng buộc M21.2).
    #[must_use]
    pub fn new(store: std::sync::Arc<dyn Store>) -> Self {
        Self { store }
    }

    /// Tra token thô ⇒ danh tính client.
    ///
    /// # Errors
    /// * [`McpAuthError::InvalidToken`] — token sai/hết hạn.
    /// * [`McpAuthError::ClientNotConfigured`] — token đúng nhưng cấu hình đã gỡ client.
    /// * [`McpAuthError::Store`] — lỗi SQLite.
    pub async fn authenticate(
        &self,
        config: &beanagent_types::Config,
        token: &str,
    ) -> Result<McpClientIdentity, McpAuthError> {
        let token = token.trim();
        // Chặn sớm định dạng sai: tránh băm và tra DB cho rác, và tránh lộ khác biệt
        // thời gian giữa "token sai hình dạng" và "token sai giá trị".
        if token.len() != MCP_TOKEN_BYTES * 2 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(McpAuthError::InvalidToken);
        }
        let now = Utc::now().to_rfc3339();
        let client = self
            .store
            .get_mcp_client(&hash_token(token), &now)
            .await?
            .ok_or(McpAuthError::InvalidToken)?;
        let name = client.name;
        let entry = config
            .mcp_client(&name)
            .ok_or_else(|| McpAuthError::ClientNotConfigured(name.clone()))?;
        Ok(McpClientIdentity {
            user_id: beanagent_types::Config::mcp_client_identity(&name),
            name,
            role: entry.role.clone(),
        })
    }
}

/// Thời điểm hết hạn của token mới theo TTL cấu hình (giờ). `0` ⇒ không hết hạn.
#[must_use]
pub fn expiry_from(ttl_hours: u32) -> String {
    if ttl_hours == 0 {
        return String::new();
    }
    let now: DateTime<Utc> = Utc::now();
    (now + Duration::hours(i64::from(ttl_hours))).to_rfc3339()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::store::MemoryStore;
    use beanagent_types::Config;
    use beanagent_types::config::{AgentConfig, McpClientConfig, RoleConfig};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn config_with_client() -> Config {
        let mut roles = BTreeMap::new();
        roles.insert(
            "mcp-client:cline".to_string(),
            "finance-readonly".to_string(),
        );
        Config {
            roles: vec![RoleConfig {
                name: "finance-readonly".into(),
                tool_tags: vec!["billing-read".into()],
                forbid_tags: vec![],
                allowed_tool_tags: vec![],
                context_budget_tokens: None,
                daily_token_budget: None,
            }],
            mcp_clients: vec![McpClientConfig {
                name: "cline".into(),
                role: "finance-readonly".into(),
            }],
            agent: AgentConfig {
                user_roles: roles,
                ..AgentConfig::default()
            },
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn valid_token_resolves_identity_and_wrong_one_is_rejected() {
        let store = Arc::new(MemoryStore::default());
        let config = config_with_client();
        let token = new_token().unwrap();
        store
            .create_mcp_client(
                &hash_token(&token),
                "cline",
                "finance-readonly",
                &Utc::now().to_rfc3339(),
                "",
            )
            .await
            .unwrap();
        let auth = McpAuth::new(store);

        let identity = auth.authenticate(&config, &token).await.unwrap();
        assert_eq!(identity.user_id, "mcp-client:cline");
        assert_eq!(identity.role, "finance-readonly");

        let other = new_token().unwrap();
        assert!(matches!(
            auth.authenticate(&config, &other).await,
            Err(McpAuthError::InvalidToken)
        ));
        assert!(matches!(
            auth.authenticate(&config, "khong-phai-hex").await,
            Err(McpAuthError::InvalidToken)
        ));
    }

    #[tokio::test]
    async fn token_of_client_removed_from_config_is_reported() {
        let store = Arc::new(MemoryStore::default());
        let token = new_token().unwrap();
        store
            .create_mcp_client(
                &hash_token(&token),
                "da-go",
                "finance-readonly",
                &Utc::now().to_rfc3339(),
                "",
            )
            .await
            .unwrap();
        let auth = McpAuth::new(store);
        let mut config = config_with_client();
        config.mcp_clients.clear();
        assert!(matches!(
            auth.authenticate(&config, &token).await,
            Err(McpAuthError::ClientNotConfigured(name)) if name == "da-go"
        ));
    }

    #[test]
    fn token_is_256_bit_hex_and_hash_is_stable() {
        let token = new_token().unwrap();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(hash_token(&token).len(), 64);
        assert_eq!(hash_token(&token), hash_token(&token));
        assert_ne!(hash_token(&token), hash_token(&new_token().unwrap()));
    }
}
