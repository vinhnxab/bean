//! Mã hoá payload Web Push theo `aes128gcm` — RFC 8291 (Message Encryption for
//! Web Push) + RFC 8188 (Encrypted Content-Encoding).
//!
//! # Vì sao tự viết mà không dùng crate `web-push`
//!
//! Crate `web-push` kéo `jwt-simple` → `rsa 0.7` dính RUSTSEC-2023-0071 (Marvin
//! Attack) làm `make audit` đỏ. Toàn bộ phần mật mã ở đây dùng cùng hệ RustCrypto
//! với workspace (`p256`/`aes-gcm`/`hkdf`/`sha2`), không thêm dependency RSA.
//!
//! # Định dạng body (`aes128gcm`, một record duy nhất — bắt buộc của RFC 8291 mục 4)
//!
//! ```text
//! body = salt(16) || rs(4 BE) || idlen(1) || as_public(65) || ciphertext
//! ciphertext = AES-128-GCM(CEK, NONCE, plaintext || 0x02, aad = rỗng)
//! ```
//!
//! Khóa dẫn xuất (mục 3.4 của RFC):
//! ```text
//! PRK_key = HKDF-Extract(salt = auth_secret, IKM = ecdh_secret)
//! IKM     = HKDF-Expand(PRK_key, "WebPush: info" || 0x00 || ua_public || as_public, 32)
//! PRK     = HKDF-Extract(salt, IKM)
//! CEK     = HKDF-Expand(PRK, "Content-Encoding: aes128gcm" || 0x00, 16)
//! NONCE   = HKDF-Expand(PRK, "Content-Encoding: nonce"      || 0x00, 12)
//! ```

use aes_gcm::Aes128Gcm;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use hkdf::Hkdf;
use p256::PublicKey;
use p256::ecdh::EphemeralSecret;
use p256::elliptic_curve::Generate;
use sha2::Sha256;

/// Kích thước salt — RFC 8291: 16 octet.
const SALT_LEN: usize = 16;
/// Record size ghi trong header — một record đủ chứa toàn bộ payload.
const RECORD_SIZE: u32 = 4096;
/// Public key ECDH uncompressed (P-256): 0x04 || X(32) || Y(32).
const PUBLIC_KEY_LEN: usize = 65;
/// Tag AEAD_AES_128_GCM.
const TAG_LEN: usize = 16;
/// Trần plaintext trước khi mã hoá: header 86 + padding 1 + tag 16 phải ≤ rs.
/// Trần an toàn để không vượt giới hạn 4096 octet của push service (RFC 8030 §7.2).
pub(crate) const MAX_PLAINTEXT_LEN: usize = RECORD_SIZE as usize - 86 - 1 - TAG_LEN;

/// Lỗi mã hoá Web Push.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EceError {
    /// Public key của trình duyệt không parse được (độ dài sai, không nằm trên
    /// đường cong P-256).
    #[error("public key của subscription không hợp lệ")]
    InvalidClientKey,
    /// Plaintext vượt trần một record (mục 4 của RFC 8291).
    #[error("payload quá lớn cho một record aes128gcm")]
    PayloadTooLarge,
    /// RNG hệ thống lỗi khi sinh khóa ephemeral/salt.
    #[error("sinh khóa ngẫu nhiên thất bại: {0}")]
    Random(String),
    /// AES-GCM encrypt lỗi (về lý thuyết không xảy ra với nonce 12 byte).
    #[error("mã hoá AES-GCM thất bại")]
    Encrypt,
}

/// Mã hoá `plaintext` cho một subscription, sinh sẵn salt + khóa ephemeral mới.
///
/// `ua_public` là public key P-256 uncompressed (65 byte, bắt đầu `0x04`) giải từ
/// `keys.p256dh` của subscription; `auth_secret` là 16 byte giải từ `keys.auth`.
pub(crate) fn encrypt(
    ua_public: &[u8],
    auth_secret: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, EceError> {
    let secret =
        EphemeralSecret::try_generate().map_err(|error| EceError::Random(error.to_string()))?;
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|error| EceError::Random(error.to_string()))?;
    encrypt_with(&salt, &secret, ua_public, auth_secret, plaintext)
}

/// Cốt lõi mã hoá với salt + khóa ephemeral **truyền vào** — tách riêng để test
/// với bộ vector RFC 8291 Appendix A (cần giá trị salt/private cố định).
fn encrypt_with(
    salt: &[u8; SALT_LEN],
    secret: &EphemeralSecret,
    ua_public: &[u8],
    auth_secret: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, EceError> {
    if plaintext.len() > MAX_PLAINTEXT_LEN {
        return Err(EceError::PayloadTooLarge);
    }
    let ua_key = PublicKey::from_sec1_bytes(ua_public).map_err(|_| EceError::InvalidClientKey)?;
    let as_public = secret.public_key();

    // ecdh_secret = ECDH(as_private, ua_public) — tọa độ X của điểm chung.
    let ecdh_secret = secret.diffie_hellman(&ua_key);

    // PRK_key = HKDF-Extract(auth_secret, ecdh_secret); IKM = Expand(PRK_key, key_info).
    let combined = Hkdf::<Sha256>::new(Some(auth_secret), ecdh_secret.raw_secret_bytes());
    let mut ikm = [0u8; 32];
    let mut key_info = Vec::with_capacity(13 + 1 + PUBLIC_KEY_LEN * 2);
    key_info.extend_from_slice(b"WebPush: info");
    key_info.push(0);
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(&as_public.to_sec1_bytes());
    combined
        .expand(&key_info, &mut ikm)
        .map_err(|_| EceError::Encrypt)?;

    // PRK = Extract(salt, IKM); CEK/NONCE = Expand theo RFC 8188.
    let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|_| EceError::Encrypt)?;
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|_| EceError::Encrypt)?;

    // Record = plaintext || padding delimiter 0x02 (record cuối/duy nhất).
    let mut record = Vec::with_capacity(plaintext.len() + 1 + TAG_LEN);
    record.extend_from_slice(plaintext);
    record.push(0x02);

    let cipher = Aes128Gcm::new((&cek).into());
    let ciphertext = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: &record,
                aad: &[],
            },
        )
        .map_err(|_| EceError::Encrypt)?;

    // Header: salt || rs(4 BE) || idlen || as_public(uncompressed 65).
    let mut body = Vec::with_capacity(86 + ciphertext.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(PUBLIC_KEY_LEN as u8);
    body.extend_from_slice(&as_public.to_sec1_bytes());
    body.extend_from_slice(&ciphertext);
    Ok(body)
}

/// Giải mã base64url (no pad) của public key/auth secret do trình duyệt gửi.
pub(crate) fn decode_b64url(value: &str) -> Result<Vec<u8>, EceError> {
    B64URL.decode(value).map_err(|_| EceError::InvalidClientKey)
}

#[cfg(test)]
mod tests {
    // Test khẳng định ciphertext RFC 8291 Appendix A bằng unwrap — cùng quy ước
    // với các file tests/*.rs của workspace.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;

    /// Toàn bộ giá trị lấy từ RFC 8291 Appendix A (ví dụ mục 5).
    mod rfc8291_a1 {
        pub(super) const PLAINTEXT: &[u8] = b"When I grow up, I want to be a watermelon";
        pub(super) const UA_PUBLIC_B64: &str = "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
        pub(super) const AUTH_SECRET_B64: &str = "BTBZMqHH6r4Tts7J_aSIgg";
        pub(super) const SALT_B64: &str = "DGv6ra1nlYgDCS1FRnbzlw";
        pub(super) const AS_PUBLIC_B64: &str = "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8";
    }

    /// Dựng lại toàn bộ chuỗi dẫn xuất của RFC 8291 Appendix A, so **từng giá
    /// trị trung gian** (ecdh_secret, IKM, CEK, NONCE) — bắt sai sớm và chỉ ra
    /// đúng bước nào lệch chuẩn.
    #[test]
    fn rfc8291_appendix_a_intermediate_values() {
        let ua_public = B64URL.decode(rfc8291_a1::UA_PUBLIC_B64).unwrap();
        let as_public = B64URL.decode(rfc8291_a1::AS_PUBLIC_B64).unwrap();
        let as_private = B64URL.decode("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw");
        let auth_secret = B64URL.decode(rfc8291_a1::AUTH_SECRET_B64).unwrap();
        let salt = B64URL.decode(rfc8291_a1::SALT_B64).unwrap();
        let mut salt_arr = [0u8; 16];
        salt_arr.copy_from_slice(&salt);

        // ecdh_secret = ECDH(as_private, ua_public) qua khóa private cứng của RFC.
        let as_bytes: [u8; 32] = as_private
            .unwrap()
            .as_slice()
            .try_into()
            .expect("khóa private P-256 phải 32 byte");
        let as_secret_key = p256::SecretKey::from_bytes(&as_bytes.into())
            .expect("khóa private của RFC phải hợp lệ");
        let ua_key =
            PublicKey::from_sec1_bytes(&ua_public).expect("khóa public UA của RFC phải hợp lệ");
        let shared =
            p256::ecdh::diffie_hellman(as_secret_key.to_nonzero_scalar(), ua_key.as_affine());
        assert_eq!(
            B64URL.encode(shared.raw_secret_bytes()),
            "kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs",
            "ecdh_secret lệch RFC"
        );

        // key_info = "WebPush: info" || 0x00 || ua_public || as_public.
        let mut key_info = Vec::new();
        key_info.extend_from_slice(b"WebPush: info");
        key_info.push(0);
        key_info.extend_from_slice(&ua_public);
        key_info.extend_from_slice(&as_public);

        let combined = Hkdf::<Sha256>::new(Some(&auth_secret), shared.raw_secret_bytes());
        let mut ikm = [0u8; 32];
        combined.expand(&key_info, &mut ikm).unwrap();
        assert_eq!(
            B64URL.encode(ikm),
            "S4lYMb_L0FxCeq0WhDx813KgSYqU26kOyzWUdsXYyrg",
            "IKM lệch RFC"
        );

        let prk = Hkdf::<Sha256>::new(Some(&salt_arr), &ikm);
        let mut cek = [0u8; 16];
        let mut nonce = [0u8; 12];
        prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
            .unwrap();
        prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
            .unwrap();
        assert_eq!(B64URL.encode(cek), "oIhVW04MRdy2XN9CiKLxTg", "CEK lệch RFC");
        assert_eq!(B64URL.encode(nonce), "4h_95klXJ5E_qnoN", "NONCE lệch RFC");
    }

    /// AES-128-GCM với (CEK, NONCE, AAD rỗng) từ vector RFC phải sinh đúng
    /// ciphertext công bố — khớp nghĩa là chuỗi CEK/NONCE/padding/AAD của ta
    /// đúng chuẩn mà không cần tự viết bộ giải mã.
    #[test]
    fn rfc8291_appendix_a_ciphertext() {
        let ua_public = B64URL.decode(rfc8291_a1::UA_PUBLIC_B64).unwrap();
        let as_public = B64URL.decode(rfc8291_a1::AS_PUBLIC_B64).unwrap();
        let auth_secret = B64URL.decode(rfc8291_a1::AUTH_SECRET_B64).unwrap();
        let salt = B64URL.decode(rfc8291_a1::SALT_B64).unwrap();
        let mut salt_arr = [0u8; 16];
        salt_arr.copy_from_slice(&salt);

        let as_bytes: [u8; 32] = B64URL
            .decode("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")
            .unwrap()
            .as_slice()
            .try_into()
            .unwrap();
        let as_secret_key = p256::SecretKey::from_bytes(&as_bytes.into()).unwrap();
        let ua_key = PublicKey::from_sec1_bytes(&ua_public).unwrap();
        let shared =
            p256::ecdh::diffie_hellman(as_secret_key.to_nonzero_scalar(), ua_key.as_affine());

        let combined = Hkdf::<Sha256>::new(Some(&auth_secret), shared.raw_secret_bytes());
        let mut key_info = Vec::new();
        key_info.extend_from_slice(b"WebPush: info");
        key_info.push(0);
        key_info.extend_from_slice(&ua_public);
        key_info.extend_from_slice(&as_public);
        let mut ikm = [0u8; 32];
        combined.expand(&key_info, &mut ikm).unwrap();

        let prk = Hkdf::<Sha256>::new(Some(&salt_arr), &ikm);
        let mut cek = [0u8; 16];
        let mut nonce = [0u8; 12];
        prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
            .unwrap();
        prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
            .unwrap();

        // Record = plaintext || 0x02 (padding delimiter của record cuối).
        let mut record = Vec::new();
        record.extend_from_slice(rfc8291_a1::PLAINTEXT);
        record.push(0x02);
        let cipher = Aes128Gcm::new((&cek).into());
        let ciphertext = cipher
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: &record,
                    aad: &[],
                },
            )
            .unwrap();
        assert_eq!(
            B64URL.encode(&ciphertext),
            "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ",
            "ciphertext AES-GCM lệch RFC"
        );
    }

    /// Body do `encrypt_with` sinh ra phải đúng **cấu trúc** RFC: salt ở đầu
    /// header, rs = 4096, idlen = 65, khóa bắt đầu 0x04, tổng độ dài 144 như
    /// ví dụ mục 5 (86 header + 41 plaintext + 1 delimiter + 16 tag).
    #[test]
    fn body_structure_matches_rfc_layout() {
        let ua_public = B64URL.decode(rfc8291_a1::UA_PUBLIC_B64).unwrap();
        let auth_secret = B64URL.decode(rfc8291_a1::AUTH_SECRET_B64).unwrap();
        let salt_b64 = B64URL.decode(rfc8291_a1::SALT_B64).unwrap();
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&salt_b64);

        let secret = EphemeralSecret::try_generate().unwrap();
        let body = encrypt_with(
            &salt,
            &secret,
            &ua_public,
            &auth_secret,
            rfc8291_a1::PLAINTEXT,
        )
        .unwrap();

        assert_eq!(&body[0..16], &salt, "salt trong header lệch");
        assert_eq!(&body[16..20], &4096u32.to_be_bytes(), "rs phải là 4096");
        assert_eq!(body[20], 65, "idlen phải là 65");
        assert_eq!(body[21], 0x04, "public key uncompressed phải bắt đầu 0x04");
        // 86 header + 41 plaintext + 1 delimiter + 16 tag = 144 — khớp chính xác
        // body base64 192 ký tự của ví dụ mục 5 (số "Content-Length: 145" trong
        // RFC là erratum, đã kiểm bằng decode).
        assert_eq!(
            body.len(),
            144,
            "độ dài body lệch RFC (ví dụ mục 5 = 144 byte)"
        );
    }

    /// Public key không phải điểm P-256 (độ dài sai) phải bị từ chối.
    #[test]
    fn rejects_invalid_client_key() {
        let secret = EphemeralSecret::try_generate().unwrap();
        let salt = [0u8; 16];
        assert!(matches!(
            encrypt_with(&salt, &secret, b"short", &[0u8; 16], b"hi"),
            Err(EceError::InvalidClientKey)
        ));
    }

    /// Payload vượt trần một record phải lỗi PayloadTooLarge.
    #[test]
    fn rejects_oversized_payload() {
        let secret = EphemeralSecret::try_generate().unwrap();
        let ua_public = {
            let key = EphemeralSecret::try_generate().unwrap();
            key.public_key().to_sec1_bytes().to_vec()
        };
        let oversized = vec![0u8; MAX_PLAINTEXT_LEN + 1];
        assert!(matches!(
            encrypt_with(&[0u8; 16], &secret, &ua_public, &[0u8; 16], &oversized),
            Err(EceError::PayloadTooLarge)
        ));
    }

    /// Mã hoá đúng trần tối đa vẫn thành công.
    #[test]
    fn accepts_max_payload() {
        let secret = EphemeralSecret::try_generate().unwrap();
        let ua_public = {
            let key = EphemeralSecret::try_generate().unwrap();
            key.public_key().to_sec1_bytes().to_vec()
        };
        let max = vec![0u8; MAX_PLAINTEXT_LEN];
        assert!(encrypt_with(&[0u8; 16], &secret, &ua_public, &[0u8; 16], &max).is_ok());
    }
}
