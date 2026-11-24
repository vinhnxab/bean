//! Khóa VAPID (Voluntary Application Server Identification) và chữ ký JWT ES256.
//!
//! VAPID (RFC 8292) là cách application server tự nhận danh tính với push service:
//! một JWT ES256 (ECDSA P-256 + SHA-256) kèm public key, gửi ở header
//! `Authorization: vapid t=<jwt>, k=<pub>`.
//!
//! # Lưu trữ khóa
//!
//! Khóa private tự sinh lần đầu và persist ở `data.dir/push-vapid.json` — quyền
//! **0600**, ghi atomic (temp + rename, mẫu `auth.rs::set_password`). Mất file chỉ
//! nghĩa là đăng ký lại subscription (trình duyệt sinh key mới khi subscribe),
//! không mất dữ liệu người dùng.

use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use p256::ecdsa::Signature;
use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::elliptic_curve::Generate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bản ghi khóa VAPID trên đĩa — chỉ chứa khóa private dạng scalar 32 byte
/// (base64url); public key dẫn xuất được, không cần lưu riêng.
#[derive(Serialize, Deserialize)]
struct VapidFile {
    /// Scalar private P-256 (32 byte, base64url no pad).
    private_key: String,
}

/// Định danh VAPID của application server: khóa ký ES256 + public key tương ứng.
pub struct VapidKeys {
    signing: p256::ecdsa::SigningKey,
    /// Public key uncompressed (65 byte, bắt đầu `0x04`) — gửi kèm header `k=`.
    public: Vec<u8>,
}

/// Lỗi khóa VAPID.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VapidError {
    /// Lỗi đĩa khi đọc/ghi file khóa.
    #[error("đọc/ghi push-vapid.json thất bại: {0}")]
    Io(String),
    /// File khóa có nhưng nội dung không parse được.
    #[error("push-vapid.json hỏng — xoá file để Bean sinh khóa mới")]
    Corrupt,
    /// RNG lỗi khi sinh khóa.
    #[error("sinh khóa VAPID thất bại: {0}")]
    Random(String),
}

impl VapidKeys {
    /// Nạp khóa từ `path`; chưa có thì sinh mới và ghi (0600, atomic).
    ///
    /// Hai tiến trình cùng gọi không hỏng nhau: file ghi bằng `create_new` —
    /// ai ghi trước thắng, bên kia đọc lại được cùng nội dung khi rename atomic.
    pub fn load_or_create(path: &Path) -> Result<Self, VapidError> {
        if let Ok(raw) = std::fs::read_to_string(path) {
            let file: VapidFile = serde_json::from_str(&raw).map_err(|_| VapidError::Corrupt)?;
            return Self::from_private_b64(&file.private_key);
        }
        let keys = Self::generate()?;
        keys.persist(path)?;
        Ok(keys)
    }

    /// Public key VAPID dạng base64url (65 byte uncompressed) — giá trị công khai,
    /// gửi cho trình duyệt lúc subscribe (tham số `applicationServerKey`).
    pub fn public_key_b64(&self) -> String {
        B64URL.encode(&self.public)
    }

    /// Header `Authorization` VAPID cho một request push đến `aud` (origin của
    /// endpoint push service). JWT sống 1 giờ — sinh mới mỗi request cho đơn giản
    /// và đúng khuyến nghị (exp ngắn, không cần cache).
    pub fn authorization_header(&self, aud: &str, now_unix: u64) -> Result<String, VapidError> {
        let claims = serde_json::json!({
            "aud": aud,
            "exp": now_unix + 3600,
            "sub": "mailto:bean@localhost",
        });
        let header = serde_json::json!({"typ": "JWT", "alg": "ES256"});
        let signing_input = format!(
            "{}.{}",
            B64URL.encode(header.to_string()),
            B64URL.encode(claims.to_string())
        );
        // ES256 = ECDSA P-256 over SHA-256(signing_input); chữ ký raw r||s 64 byte.
        let digest = Sha256::digest(signing_input.as_bytes());
        let signature: Signature = self
            .signing
            .sign_prehash(&digest)
            .map_err(|_| VapidError::Random("ký JWT ES256 thất bại".into()))?;
        let sig_b64 = B64URL.encode(signature.to_bytes().as_slice());
        Ok(format!(
            "vapid t={signing_input}.{sig_b64}, k={}",
            self.public_key_b64()
        ))
    }

    fn from_private_b64(private_key: &str) -> Result<Self, VapidError> {
        let scalar = B64URL
            .decode(private_key)
            .map_err(|_| VapidError::Corrupt)?;
        let bytes: [u8; 32] = scalar
            .as_slice()
            .try_into()
            .map_err(|_| VapidError::Corrupt)?;
        let signing =
            p256::ecdsa::SigningKey::from_bytes(&bytes.into()).map_err(|_| VapidError::Corrupt)?;
        Ok(Self::from_signing(signing))
    }

    fn generate() -> Result<Self, VapidError> {
        // `SigningKey::random()`/`generate()` bị cấm (panic theo lint workspace) —
        // dùng `Generate::try_generate()` trả Result.
        let signing = p256::ecdsa::SigningKey::try_generate()
            .map_err(|error| VapidError::Random(error.to_string()))?;
        Ok(Self::from_signing(signing))
    }

    fn from_signing(signing: p256::ecdsa::SigningKey) -> Self {
        // Public key dẫn xuất trực tiếp từ SigningKey: `to_sec1_bytes()` mã hóa
        // uncompressed 65 byte vì p256 đặt `COMPRESS_POINTS = false` — không cần
        // đi vòng qua `SecretKey` (bắt buộc `.expect()` ở đường không thể lỗi).
        let public = signing.verifying_key().to_sec1_bytes().into_vec();
        Self { signing, public }
    }

    fn persist(&self, path: &Path) -> Result<(), VapidError> {
        let data_dir = path
            .parent()
            .ok_or_else(|| VapidError::Io("đường dẫn khóa thiếu thư mục cha".into()))?;
        std::fs::create_dir_all(data_dir).map_err(|error| VapidError::Io(error.to_string()))?;
        let temp = data_dir.join(".push-vapid.json.tmp");
        let _ = std::fs::remove_file(&temp);
        let content = serde_json::to_string(&VapidFile {
            private_key: B64URL.encode(self.signing.to_bytes()),
        })
        .map_err(|_| VapidError::Corrupt)?;
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|error| VapidError::Io(error.to_string()))?;
        if let Err(error) = file
            .write_all(content.as_bytes())
            .and_then(|_| file.sync_all())
        {
            let _ = std::fs::remove_file(&temp);
            return Err(VapidError::Io(error.to_string()));
        }
        drop(file);
        if let Err(error) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(VapidError::Io(error.to_string()));
        }
        if let Err(error) = set_owner_only(path) {
            return Err(VapidError::Io(error));
        }
        Ok(())
    }
}

/// Chặn quyền file khóa về 0600 trên Unix (phòng trường hợp umist lạ).
#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Đường dẫn file khóa VAPID từ data.dir.
pub fn vapid_keys_path(data_dir: &Path) -> PathBuf {
    data_dir.join("push-vapid.json")
}

#[cfg(test)]
mod tests {
    // Test của push dựng khóa/JSON cố định và khẳng định bằng unwrap/expect —
    // cùng quy ước với các file tests/*.rs của workspace.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use p256::ecdsa::signature::Verifier;

    fn temp_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = vapid_keys_path(dir.path());
        (dir, path)
    }

    /// Sinh lần đầu tạo file, lần sau nạp cùng khóa public.
    #[test]
    fn load_or_create_is_stable() {
        let (_dir, path) = temp_dir();
        let first = VapidKeys::load_or_create(&path).unwrap();
        let second = VapidKeys::load_or_create(&path).unwrap();
        assert_eq!(first.public_key_b64(), second.public_key_b64());
        // Public key uncompressed 65 byte, bắt đầu 0x04.
        let raw = B64URL.decode(first.public_key_b64()).unwrap();
        assert_eq!(raw.len(), 65);
        assert_eq!(raw[0], 0x04);
    }

    /// File khóa có quyền 0600 trên Unix — khóa private không đọc được bởi user khác.
    #[test]
    #[cfg(unix)]
    fn key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = temp_dir();
        let _ = VapidKeys::load_or_create(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "file khóa VAPID phải 0600");
    }

    /// File hỏng (không phải JSON hợp lệ) trả Corrupt — handler trả lỗi rõ ràng,
    /// không âm thầm ghi đè khóa (khóa cũ còn dùng bởi subscription đã đăng ký).
    #[test]
    fn corrupt_file_is_reported() {
        let (_dir, path) = temp_dir();
        std::fs::write(&path, "không phải json").unwrap();
        assert!(matches!(
            VapidKeys::load_or_create(&path),
            Err(VapidError::Corrupt)
        ));
    }

    /// JWT ES256: 3 phân đoạn, header/payload base64url giải được, chữ ký
    /// **xác minh thành công** bằng chính public key của khóa — tức signature
    /// đúng định dạng raw r||s và băm SHA-256 đúng chuẩn.
    #[test]
    fn jwt_verifies_with_own_public_key() {
        let keys = VapidKeys::generate().unwrap();
        let header = keys
            .authorization_header("https://push.example.net", 1_700_000_000)
            .unwrap();

        // Header chuẩn: "vapid t=<jwt>, k=<pub>".
        let (scheme, rest) = header.split_once(' ').expect("header phải có dấu cách");
        assert_eq!(scheme, "vapid");
        let (t_part, k_part) = rest.split_once(", ").expect("thiếu phần k=");
        let jwt = t_part.strip_prefix("t=").expect("thiếu t=");
        assert!(k_part.starts_with("k="));
        assert_eq!(
            k_part.strip_prefix("k=").unwrap(),
            keys.public_key_b64(),
            "k= phải đúng public key"
        );

        let segments: Vec<&str> = jwt.split('.').collect();
        assert_eq!(segments.len(), 3, "JWT phải có 3 phân đoạn");

        let header_json = String::from_utf8(B64URL.decode(segments[0]).unwrap()).unwrap();
        assert!(
            header_json.contains("ES256"),
            "header JWT phải khai báo ES256"
        );
        let claims: serde_json::Value =
            serde_json::from_slice(&B64URL.decode(segments[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://push.example.net");
        assert_eq!(claims["exp"], 1_700_000_000 + 3600);
        assert_eq!(claims["sub"], "mailto:bean@localhost");

        // Chữ ký: ES256 = ECDSA(P-256) over SHA-256; raw r||s 64 byte.
        let signature_bytes = B64URL.decode(segments[2]).unwrap();
        assert_eq!(signature_bytes.len(), 64, "chữ ký ES256 raw phải 64 byte");
        let signature = Signature::from_slice(&signature_bytes).unwrap();
        let signing_input = format!("{}.{}", segments[0], segments[1]);
        keys.signing
            .verifying_key()
            .verify(signing_input.as_bytes(), &signature)
            .expect("chữ ký JWT phải xác minh được bằng public key");
    }

    /// Hai lần ký sinh JWT khác nhau (giờ thực tế sẽ khác exp) — kiểm tra hàm
    /// không trả chuỗi tĩnh nào.
    #[test]
    fn jwt_has_expiry_one_hour() {
        let keys = VapidKeys::generate().unwrap();
        let base = 1_700_000_000u64;
        let a = keys
            .authorization_header("https://push.example.net", base)
            .unwrap();
        let b = keys
            .authorization_header("https://push.example.net", base + 60)
            .unwrap();
        assert_ne!(a, b);
    }
}
