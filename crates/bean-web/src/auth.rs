//! Argon2id password file, 256-bit session tokens và rate-limit đăng nhập.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use bean_memory::{Store, WebSessionInfo};
use bean_security::ratelimit::RateLimiter;
use bean_security::{AuditLog, entry_now};
use bean_types::Config;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Tên file chứa hash mật khẩu.
pub const AUTH_FILE: &str = "auth.toml";
/// Định danh web mặc định khi `[web].user_id` không được khai báo.
///
/// Re-export từ `bean-types` để crate web và crate khác dùng **cùng một** hằng
/// số — trước đây hằng số này nằm ở đây nên không thể làm mặc định serde trong
/// `Config` mà không tạo phụ thuộc vòng.
pub const WEB_USER: &str = bean_types::config::DEFAULT_WEB_USER;
/// Số byte token phiên đăng nhập.
pub const SESSION_TOKEN_BYTES: usize = 32;

/// File auth chỉ chứa PHC string, không chứa plaintext.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthFile {
    password_hash: String,
    /// Thời điểm đổi mật khẩu gần nhất (RFC3339 UTC). `None` = chưa đổi /
    /// file cũ. Session tạo trước mốc này bị từ chối trong `authenticate_token`.
    #[serde(default)]
    password_changed_at: Option<String>,
}

/// Lỗi auth nội bộ, không chứa password/token.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("không tìm thấy auth.toml; hãy chạy `bean auth set-password`")]
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
    password_hash: RwLock<String>,
    password_changed_at: RwLock<Option<String>>,
    store: Arc<dyn Store>,
    user_id: String,
    data_dir: PathBuf,
    session_ttl: Duration,
    secure_cookie: bool,
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
        Self::from_hash(file.password_hash, file.password_changed_at, config, store)
    }

    /// Tạo service từ hash đã parse.
    pub fn from_hash(
        password_hash: String,
        password_changed_at: Option<String>,
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
                password_hash: RwLock::new(password_hash),
                password_changed_at: RwLock::new(password_changed_at),
                store,
                user_id: config.web.user_id.clone(),
                data_dir: config.data.dir.clone(),
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
        let hash = hash_token(token);
        let info = self
            .inner
            .store
            .get_web_session(&hash, &Utc::now().to_rfc3339())
            .await
            .map_err(|error| AuthError::Io(error.to_string()))?;
        let info = info.ok_or(AuthError::InvalidCredentials)?;

        // Q1: session tạo trước `password_changed_at` bị thu hồi.
        let changed_at = self
            .inner
            .password_changed_at
            .read()
            .map_err(|_| AuthError::Invalid)?
            .clone();
        if let Some(changed) = changed_at
            && let Some(info_created) = parse_timestamp(&info.created_at)
            && let Some(changed_dt) = parse_timestamp(&changed)
            && info_created < changed_dt
        {
            return Err(AuthError::InvalidCredentials);
        }

        let _ = self
            .inner
            .store
            .touch_web_session(&hash, &now_rfc3339())
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
        let parsed = argon2::PasswordHash::new(
            &self
                .inner
                .password_hash
                .read()
                .map_err(|_| AuthError::Invalid)?
                .clone(),
        )
        .map_err(|_| AuthError::Invalid)?;
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
        format!("bean_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
    }

    /// Xác minh mật khẩu hiện tại rồi đặt mật khẩu mới và cấp phiên mới cho người gọi.
    ///
    /// Q1: session cũ của mọi người bị thu hồi qua `password_changed_at`;
    /// chỉ session mới của người gọi đổi mật khẩu được cấp lại ngay.
    pub async fn change_password(
        &self,
        token: &str,
        current_password: &str,
        new_password: &str,
        audit: Option<&AuditLog>,
        ip: IpAddr,
    ) -> Result<LoginSession, AuthError> {
        let current_hash = self
            .inner
            .password_hash
            .read()
            .map_err(|_| AuthError::Invalid)?
            .clone();
        let parsed = argon2::PasswordHash::new(&current_hash).map_err(|_| AuthError::Invalid)?;
        if parsed.algorithm.as_str() != "argon2id" {
            return Err(AuthError::Invalid);
        }
        let valid = Argon2::default()
            .verify_password(current_password.as_bytes(), &parsed)
            .is_ok();
        if !valid {
            record_auth_event(
                audit,
                ip,
                "auth_change_password",
                false,
                Some("invalid_credentials"),
            );
            return Err(AuthError::InvalidCredentials);
        }
        // Ghi hash mới + mốc thời điểm thu hồi, rồi xóa toàn bộ session cũ.
        let changed_at = now_rfc3339();
        set_password_with_revoke(
            &self.inner.data_dir,
            new_password,
            Some(changed_at.clone()),
            self.inner.store.clone(),
        )
        .await?;
        // Cập nhật hash trong memory.
        let new_hash = read_password_hash(&self.inner.data_dir)?;
        *self
            .inner
            .password_hash
            .write()
            .map_err(|_| AuthError::Invalid)? = new_hash;
        *self
            .inner
            .password_changed_at
            .write()
            .map_err(|_| AuthError::Invalid)? = Some(changed_at);

        // Cấp session mới cho người gọi (token từ cookie hiện tại).
        let token_hash = hash_token(token);
        let created = Utc::now();
        let expires = created
            + chrono::Duration::from_std(self.inner.session_ttl)
                .map_err(|error| AuthError::Io(error.to_string()))?;
        self.inner
            .store
            .create_web_session(
                token_hash.to_vec(),
                &self.inner.user_id,
                &created.to_rfc3339(),
                &expires.to_rfc3339(),
            )
            .await
            .map_err(|error| AuthError::Io(error.to_string()))?;

        record_auth_event(audit, ip, "auth_change_password", true, None);
        Ok(LoginSession {
            token: token.to_string(),
            expires_at: expires.to_rfc3339(),
        })
    }

    /// Xác minh mật khẩu cho hành động nhạy cảm (step-up, D19.5): ghi/xoá secret qua
    /// UI yêu cầu mật khẩu hiện tại ngay cả khi đã có cookie phiên — session bị đánh
    /// cắp không đủ để đọc/xoá key.
    ///
    /// Dùng **chung** [`RateLimiter`] với `login` (cùng khoá `ip:<addr>`): dò mật khẩu
    /// qua endpoint step-up bị khoá đúng như dò qua đăng nhập — 5 lần sai trong 60
    /// giây thì khoá tăng dần. Đúng mật khẩu xoá bộ đếm (người thật đã chứng minh).
    /// Đồng bộ (không `await`): phần verify argon2id vốn chạy trên thread hiện tại
    /// giống `login`.
    ///
    /// # Errors
    /// [`AuthError::RateLimited`] khi vượt giới hạn, [`AuthError::InvalidCredentials`]
    /// khi mật khẩu sai, [`AuthError::Invalid`] khi hash file hỏng.
    pub fn verify_step_up(
        &self,
        password: &str,
        ip: IpAddr,
        audit: Option<&AuditLog>,
    ) -> Result<(), AuthError> {
        if let Err(error) = self.check_limit(ip) {
            if matches!(error, AuthError::RateLimited(_)) {
                record_auth_event(audit, ip, "auth_step_up", false, Some("rate_limited"));
            }
            return Err(error);
        }
        let parsed = argon2::PasswordHash::new(
            &self
                .inner
                .password_hash
                .read()
                .map_err(|_| AuthError::Invalid)?
                .clone(),
        )
        .map_err(|_| AuthError::Invalid)?;
        // Phải là Argon2id, không chấp nhận thuật toán khác từ file bị sửa.
        if parsed.algorithm.as_str() != "argon2id" {
            return Err(AuthError::Invalid);
        }
        let valid = Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();
        if !valid {
            self.record_failure(ip);
            record_auth_event(
                audit,
                ip,
                "auth_step_up",
                false,
                Some("invalid_credentials"),
            );
            return Err(AuthError::InvalidCredentials);
        }
        self.clear_attempts(ip);
        record_auth_event(audit, ip, "auth_step_up", true, None);
        Ok(())
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

/// Đọc hash mật khẩu hiện tại từ `auth.toml`.
fn read_password_hash(data_dir: &Path) -> Result<String, AuthError> {
    let raw =
        fs::read_to_string(auth_file_path(data_dir)).map_err(|e| AuthError::Io(e.to_string()))?;
    let file: AuthFile = toml::from_str(&raw).map_err(|_| AuthError::Invalid)?;
    Ok(file.password_hash)
}

/// Hash password bằng Argon2id và ghi atomically, quyền 0600 trên Unix.
///
/// `password_changed_at` được ghi vào `auth.toml` khi có (UI đổi mật khẩu)
/// để thu hồi session cũ qua mốc thời điểm. `None` = chỉ ghi hash (CLI `set-password`).
pub fn set_password(
    data_dir: &Path,
    password: &str,
    password_changed_at: Option<&str>,
) -> Result<(), AuthError> {
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
        password_changed_at: password_changed_at.map(str::to_string),
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

/// Ghi mật khẩu mới, ghi mốc thu hồi (nếu có), rồi vô hiệu toàn bộ phiên.
///
/// `password_changed_at` = `Some(ts)`: UI đổi mật khẩu — session tạo trước `ts`
/// bị từ chối trong `authenticate_token`.
/// `password_changed_at` = `None`: CLI `auth set-password` — chỉ xóa session,
/// không ghi mốc (tùy kiểm tra hash thay thế).
pub async fn set_password_with_revoke(
    data_dir: &Path,
    password: &str,
    password_changed_at: Option<String>,
    store: Arc<dyn Store>,
) -> Result<(), AuthError> {
    // Revoke first: nếu ghi file thất bại, người dùng bị đăng xuất nhưng không còn
    // cookie cũ có thể xác thực.
    store
        .delete_all_web_sessions()
        .await
        .map_err(|error| AuthError::Io(error.to_string()))?;
    let changed_at_ref = password_changed_at.as_deref();
    set_password(data_dir, password, changed_at_ref)
}

/// Ghi mật khẩu mới rồi xóa toàn bộ session (dùng cho `auth set-password` CLI).
/// Giữ lại để tương thích ngược với gọi cũ.
pub async fn set_password_and_revoke_sessions(
    data_dir: &Path,
    password: &str,
    store: Arc<dyn Store>,
) -> Result<(), AuthError> {
    set_password_with_revoke(data_dir, password, None, store).await
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
