//! Argon2id password file, 256-bit session tokens và rate-limit đăng nhập.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use beanagent_memory::{Store, WebSessionInfo};
use beanagent_security::ratelimit::RateLimiter;
use beanagent_security::{AuditLog, entry_now};
use beanagent_types::Config;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Tên file chứa hash mật khẩu.
pub const AUTH_FILE: &str = "auth.toml";
/// User web một người dùng của v1.
pub const WEB_USER: &str = "web:admin";
/// Số byte token phiên đăng nhập.
pub const SESSION_TOKEN_BYTES: usize = 32;

/// File auth chỉ chứa PHC string, không chứa plaintext.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthFile {
    password_hash: String,
}

/// Lỗi auth nội bộ, không chứa password/token.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("không tìm thấy auth.toml; hãy chạy `BeanAgent auth set-password`")]
    Missing,
    #[error("auth.toml không hợp lệ")]
    Invalid,
    #[error("không thể đọc/ghi auth.toml: {0}")]
    Io(String),
    #[error("auth.toml phải có quyền 0600")]
    Permissions,
    #[error("không sinh được token phiên an toàn")]
    Random,
    #[error("mật khẩu không đúng")]
    InvalidCredentials,
    #[error("đã vượt giới hạn đăng nhập; thử lại sau {0} giây")]
    RateLimited(u64),
}

/// Kết quả đăng nhập thành công.
#[derive(Debug, Clone)]
pub struct LoginSession {
    pub token: String,
    pub expires_at: String,
}

struct AuthInner {
    password_hash: String,
    store: Arc<dyn Store>,
    user_id: String,
    session_ttl: Duration,
    secure_cookie: bool,
    /// Khoá theo IP dùng chung với MCP server (`beanagent_security::ratelimit`) —
    /// thuật toán khoá nằm ở đúng một chỗ (K24).
    attempts: RateLimiter,
}

/// Dịch vụ xác thực; DB chỉ lưu hash token.
#[derive(Clone)]
pub struct AuthService {
    inner: Arc<AuthInner>,
}

impl std::fmt::Debug for AuthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthService")
            .field("user_id", &self.inner.user_id)
            .field("session_ttl", &self.inner.session_ttl)
            .finish_non_exhaustive()
    }
}

impl AuthService {
    /// Nạp hash từ `data.dir/auth.toml` và kiểm tra quyền file.
    pub fn load(config: &Config, store: Arc<dyn Store>) -> Result<Self, AuthError> {
        let path = auth_file_path(&config.data.dir);
        let metadata = fs::metadata(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AuthError::Missing
            } else {
                AuthError::Io(error.to_string())
            }
        })?;
        check_permissions(&metadata)?;
        let raw = fs::read_to_string(&path).map_err(|error| AuthError::Io(error.to_string()))?;
        let file: AuthFile = toml::from_str(&raw).map_err(|_| AuthError::Invalid)?;
        argon2::PasswordHash::new(&file.password_hash).map_err(|_| AuthError::Invalid)?;
        Self::from_hash(file.password_hash, config, store)
    }

    /// Tạo service từ hash đã parse.
    pub fn from_hash(
        password_hash: String,
        config: &Config,
        store: Arc<dyn Store>,
    ) -> Result<Self, AuthError> {
        // Chỉ chấp nhận đúng Argon2id, không tin thuật toán ghi trong file.
        let parsed = argon2::PasswordHash::new(&password_hash).map_err(|_| AuthError::Invalid)?;
        if parsed.algorithm.as_str() != "argon2id" {
            return Err(AuthError::Invalid);
        }
        Ok(Self {
            inner: Arc::new(AuthInner {
                password_hash,
                store,
                user_id: WEB_USER.to_string(),
                session_ttl: Duration::from_secs(
                    u64::from(config.web.session_ttl_hours).saturating_mul(3600),
                ),
                secure_cookie: config.web.public_origin.starts_with("https://"),
                attempts: RateLimiter::default(),
            }),
        })
    }

    /// Xác thực cookie và trả user ID.
    pub async fn authenticate_token(&self, token: &str) -> Result<WebSessionInfo, AuthError> {
        if token.len() != SESSION_TOKEN_BYTES * 2 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AuthError::InvalidCredentials);
        }
        let hash = hash_token(token);
        let now = now_rfc3339();
        let info = self
            .inner
            .store
            .get_web_session(&hash, &now)
            .await
            .map_err(|error| AuthError::Io(error.to_string()))?
            .ok_or(AuthError::InvalidCredentials)?;
        let _ = self
            .inner
            .store
            .touch_web_session(&hash, &now)
            .await
            .map_err(|error| AuthError::Io(error.to_string()))?;
        Ok(info)
    }
    /// Đăng nhập; IP là nguồn rate-limit đã được chuẩn hoá ở HTTP layer.
    pub async fn login(
        &self,
        password: &str,
        ip: IpAddr,
        audit: Option<&AuditLog>,
    ) -> Result<LoginSession, AuthError> {
        if let Err(error) = self.check_limit(ip) {
            if matches!(error, AuthError::RateLimited(_)) {
                record_auth_event(audit, ip, "auth_login", false, Some("rate_limited"));
            }
            return Err(error);
        }
        let parsed =
            argon2::PasswordHash::new(&self.inner.password_hash).map_err(|_| AuthError::Invalid)?;
        // Phải là Argon2id, không chấp nhận thuật toán khác từ file bị sửa.
        if parsed.algorithm.as_str() != "argon2id" {
            return Err(AuthError::Invalid);
        }
        let valid = Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();
        if !valid {
            self.record_failure(ip);
            record_auth_event(audit, ip, "auth_login", false, Some("invalid_credentials"));
            return Err(AuthError::InvalidCredentials);
        }
        self.clear_attempts(ip);
        let token = random_token()?;
        let created = Utc::now();
        let expires = created
            + chrono::Duration::from_std(self.inner.session_ttl)
                .map_err(|error| AuthError::Io(error.to_string()))?;
        self.inner
            .store
            .create_web_session(
                hash_token(&token).to_vec(),
                &self.inner.user_id,
                &created.to_rfc3339(),
                &expires.to_rfc3339(),
            )
            .await
            .map_err(|error| AuthError::Io(error.to_string()))?;
        record_auth_event(audit, ip, "auth_login", true, None);
        Ok(LoginSession {
            token,
            expires_at: expires.to_rfc3339(),
        })
    }

    /// Xoá một phiên đăng nhập theo token.
    pub async fn logout(
        &self,
        token: &str,
        audit: Option<&AuditLog>,
        ip: IpAddr,
    ) -> Result<(), AuthError> {
        self.inner
            .store
            .delete_web_session(&hash_token(token))
            .await
            .map_err(|error| AuthError::Io(error.to_string()))?;
        record_auth_event(audit, ip, "auth_logout", true, None);
        Ok(())
    }

    /// Header Set-Cookie chuẩn, không dùng Domain.
    pub fn session_cookie(&self, session: &LoginSession) -> String {
        let secure = if self.inner.secure_cookie {
            "; Secure"
        } else {
            ""
        };
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{secure}",
            crate::api_types::SESSION_COOKIE,
            session.token,
            self.inner.session_ttl.as_secs(),
        )
    }

    /// Header cookie hết hạn khi logout.
    pub fn expired_cookie(&self) -> String {
        let secure = if self.inner.secure_cookie {
            "; Secure"
        } else {
            ""
        };
        format!("beanagent_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
    }

    /// Khoá theo IP. Tên khoá có tiền tố `ip:` để đọc log là biết đang khoá theo IP
    /// (dùng chung `RateLimiter` với MCP nên khoá còn có thể là `token:`).
    fn check_limit(&self, ip: IpAddr) -> Result<(), AuthError> {
        self.inner
            .attempts
            .check(&ip_key(ip))
            .map_err(|wait| AuthError::RateLimited(wait.as_secs()))
    }

    fn record_failure(&self, ip: IpAddr) {
        self.inner.attempts.record_failure(&ip_key(ip));
    }

    fn clear_attempts(&self, ip: IpAddr) {
        self.inner.attempts.clear(&ip_key(ip));
    }
}

/// Khoá IP của `AuthService` — cùng định dạng với khoá của MCP server.
fn ip_key(ip: IpAddr) -> String {
    format!("ip:{ip}")
}

/// Đường dẫn auth.toml từ data directory đã được Config chuẩn hoá.
#[must_use]
pub fn auth_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(AUTH_FILE)
}

/// Hash password bằng Argon2id và ghi atomically, quyền 0600 trên Unix.
pub fn set_password(data_dir: &Path, password: &str) -> Result<(), AuthError> {
    if password.len() < 12 {
        return Err(AuthError::InvalidCredentials);
    }
    let hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
        .hash_password(password.as_bytes())
        .map_err(|_| AuthError::Invalid)?
        .to_string();
    fs::create_dir_all(data_dir).map_err(|error| AuthError::Io(error.to_string()))?;
    let path = auth_file_path(data_dir);
    let temp = data_dir.join(".auth.toml.tmp");
    let _ = fs::remove_file(&temp);
    let content = toml::to_string(&AuthFile {
        password_hash: hash,
    })
    .map_err(|_| AuthError::Invalid)?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|error| AuthError::Io(error.to_string()))?;
    if let Err(error) = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(&temp);
        return Err(AuthError::Io(error.to_string()));
    }
    drop(file);
    if let Err(error) = fs::rename(&temp, &path) {
        let _ = fs::remove_file(&temp);
        return Err(AuthError::Io(error.to_string()));
    }
    set_permissions(&path)
}

/// Ghi mật khẩu mới rồi vô hiệu toàn bộ phiên đăng nhập cũ.
///
/// Hàm này dùng cho lệnh `auth set-password`: đổi mật khẩu không được để lại
/// cookie cũ vẫn xác thực được.
pub async fn set_password_and_revoke_sessions(
    data_dir: &Path,
    password: &str,
    store: Arc<dyn Store>,
) -> Result<(), AuthError> {
    // Revoke first: nếu ghi file thất bại, người dùng bị đăng xuất nhưng không còn
    // cookie cũ có thể xác thực; tuyệt đối không để trường hợp đổi hash thành công
    // nhưng session cũ vẫn sống.
    store
        .delete_all_web_sessions()
        .await
        .map_err(|error| AuthError::Io(error.to_string()))?;
    set_password(data_dir, password)
}

fn set_permissions(path: &Path) -> Result<(), AuthError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| AuthError::Io(error.to_string()))?;
    }
    let _ = path;
    Ok(())
}

fn check_permissions(metadata: &fs::Metadata) -> Result<(), AuthError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(AuthError::Permissions);
        }
    }
    let _ = metadata;
    Ok(())
}

fn hash_token(token: &str) -> [u8; 32] {
    let digest = Sha256::digest(token.as_bytes());
    let mut result = [0_u8; 32];
    result.copy_from_slice(&digest);
    result
}

fn random_token() -> Result<String, AuthError> {
    let mut bytes = [0_u8; SESSION_TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::Random)?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}").map_err(|_| AuthError::Random)?;
    }
    Ok(token)
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn record_auth_event(
    audit: Option<&AuditLog>,
    ip: IpAddr,
    event: &str,
    success: bool,
    error: Option<&str>,
) {
    let Some(log) = audit else { return };
    let mut entry = entry_now(0, "web", event, &serde_json::json!({"ip": ip.to_string()}));
    entry.ok = Some(success);
    entry.decision = if success { "allow" } else { "deny" };
    entry.decided_by = "web".into();
    entry.error = error.map(str::to_string);
    if let Err(error) = log.record(&entry) {
        tracing::warn!(error = %error, "không ghi được audit đăng nhập");
    }
}

/// Parse thời điểm audit/telemetry.
#[must_use]
pub fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}
