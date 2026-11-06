//! `data.dir/secrets.env` — nơi lưu secret duy nhất của Bean (M29, D19.1).
//!
//! Định dạng: `NAME=value`, một dòng một khoá; dòng trống và dòng bắt đầu bằng `#`
//! được bỏ qua. Tên phải đúng dạng tên biến môi trường; giá trị tối đa
//! [`MAX_VALUE_BYTES`] byte và **không** chứa ký tự điều khiển (kể cả xuống dòng) —
//! vi phạm bị từ chối, không tự cắt âm thầm.
//!
//! An toàn khi ghi:
//! * tmp file **tạo ngay với quyền `0600`** (không `chmod` sau) → `fsync` → `rename`
//!   cùng thư mục — ghi hỏng giữa chừng không mất file cũ;
//! * read-modify-write dưới **khoá file** (`<file>.env.lock` qua `fd-lock`) để CLI và
//!   server không ghi chồng mất dữ liệu của nhau;
//! * file hiện tại parse lỗi ⇒ **không ghi**, giữ nguyên dữ liệu cũ.
//!
//! Secret sống trong bộ nhớ dưới dạng `secrecy::SecretString`; Bean không bao giờ
//! `set_var` vào môi trường tiến trình — tiến trình con không thừa hưởng.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretString};

use crate::error::SettingsError;
use crate::slot::{SecretSlotRegistry, is_env_name};

/// Giới hạn độ dài một giá trị secret (byte).
pub const MAX_VALUE_BYTES: usize = 4096;

/// Kho secret gắn với một file `secrets.env`.
///
/// Bản thân struct chỉ giữ đường dẫn — mọi thao tác đọc/ghi đụng đĩa trực tiếp, nên
/// clone/chia sẻ giữa CLI và server không có trạng thái cũ đọng lại.
#[derive(Debug, Clone)]
pub struct SecretStore {
    path: PathBuf,
}

/// Ảnh chụp chỉ-đọc của các entry đang có — dùng để resolve secret lúc chạy.
///
/// `Debug` của `SecretString` (secrecy) tự redact, nên in nguyên snapshot ra log cũng
/// không lộ giá trị; tuy nhiên vẫn **không** có lý do gì log snapshot.
#[derive(Debug, Clone, Default)]
pub struct SecretSnapshot {
    entries: Vec<(String, SecretString)>,
}

impl SecretStore {
    /// Kho gắn với đường dẫn file (thường `data.dir/secrets.env`).
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Đường dẫn file của kho.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Đọc ảnh chụp hiện tại; file chưa tồn tại ⇒ snapshot rỗng (lần chạy đầu).
    ///
    /// # Errors
    /// [`SettingsError::Parse`] khi file sai định dạng; [`SettingsError::Io`] khi
    /// không đọc được đĩa.
    pub fn snapshot(&self) -> Result<SecretSnapshot, SettingsError> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SecretSnapshot::default());
            }
            Err(error) => {
                return Err(SettingsError::Io {
                    path: self.path.clone(),
                    source: error,
                });
            }
        };
        parse_entries(&self.path, &raw)
    }

    /// Đặt/ghi đè giá trị một slot. Tên phải thuộc `registry` (mục 4.1 — ghi tên lạ bị
    /// từ chối để `secrets.env` không thành đường chèn biến môi trường); giá trị phải
    /// đạt ràng buộc của [`MAX_VALUE_BYTES`] và không chứa ký tự điều khiển.
    ///
    /// # Errors
    /// [`SettingsError::UnknownSlot`] khi tên lạ; [`SettingsError::InvalidValue`] khi
    /// giá trị sai; [`SettingsError::Parse`] khi file hiện tại hỏng (không ghi đè lên
    /// dữ liệu cũ); [`SettingsError::Io`] khi không ghi được đĩa.
    pub fn set(
        &self,
        registry: &SecretSlotRegistry,
        name: &str,
        value: SecretString,
    ) -> Result<(), SettingsError> {
        if registry.slot(name).is_none() {
            return Err(SettingsError::UnknownSlot {
                name: name.to_string(),
            });
        }
        validate_value(name, value.expose_secret())?;
        self.modify(|entries| {
            let owned = value.expose_secret().to_string();
            if let Some(entry) = entries.iter_mut().find(|(existing, _)| existing == name) {
                entry.1 = SecretString::from(owned);
            } else {
                entries.push((name.to_string(), SecretString::from(owned)));
            }
        })
    }

    /// Xoá một slot; trả `true` khi có entry bị xoá, `false` khi slot vốn trống.
    ///
    /// # Errors
    /// [`SettingsError::UnknownSlot`] khi tên lạ; như [`Self::set`] cho I/O/parse.
    pub fn remove(&self, registry: &SecretSlotRegistry, name: &str) -> Result<bool, SettingsError> {
        if registry.slot(name).is_none() {
            return Err(SettingsError::UnknownSlot {
                name: name.to_string(),
            });
        }
        let mut removed = false;
        self.modify(|entries| {
            let before = entries.len();
            entries.retain(|(existing, _)| existing != name);
            removed = before != entries.len();
        })?;
        Ok(removed)
    }

    /// Read-modify-write dưới khoá file: `change` sửa danh sách entry trong bộ nhớ,
    /// kết quả được ghi atomic. File hiện tại parse lỗi ⇒ không ghi gì cả.
    fn modify(
        &self,
        change: impl FnOnce(&mut Vec<(String, SecretString)>),
    ) -> Result<(), SettingsError> {
        let parent = self
            .path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        fs::create_dir_all(&parent).map_err(|error| SettingsError::Io {
            path: parent.clone(),
            source: error,
        })?;
        ensure_private_dir(&parent)?;

        let mut lock = open_lock_file(&self.path)?;
        // `write()` cần `&mut` — khoá chờ tới khi CLI/server khác buông.
        let guard = lock.write().map_err(|error| SettingsError::Io {
            path: self.path.clone(),
            source: error,
        })?;
        // Đọc lại dưới khoá — tiến trình khác có thể vừa ghi xong.
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(SettingsError::Io {
                    path: self.path.clone(),
                    source: error,
                });
            }
        };
        let snapshot = parse_entries(&self.path, &raw)?;
        let mut entries = snapshot.entries;
        change(&mut entries);
        write_atomic(&self.path, &serialize_entries(&entries))?;
        drop(guard);
        Ok(())
    }
}

impl SecretSnapshot {
    /// Giá trị của một slot; `None` khi chưa đặt.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<SecretString> {
        self.entries
            .iter()
            .find(|(existing, _)| existing == name)
            .map(|(_, value)| value.clone())
    }

    /// Giá trị dạng `String` — chỉ làm cầu nối cho closure `resolve_*_with` của
    /// `bean-types`, vốn bọc lại thành `SecretString` ngay lập tức.
    #[must_use]
    pub fn lookup_str(&self, name: &str) -> Option<String> {
        self.get(name)
            .map(|value| value.expose_secret().to_string())
    }

    /// `true` khi slot đã có giá trị trong file.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|(existing, _)| existing == name)
    }

    /// Tên các slot đang có giá trị (theo thứ tự trong file).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(name, _)| name.as_str())
    }

    /// `true` khi file trống/chưa tồn tại.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Parse nội dung file thành snapshot. Mọi lỗi nêu **số dòng và tên khoá**, không
/// bao giờ lặp lại giá trị.
fn parse_entries(path: &Path, raw: &str) -> Result<SecretSnapshot, SettingsError> {
    let mut entries: Vec<(String, SecretString)> = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim_start().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let line_no = index + 1;
        let Some((name, value)) = line.split_once('=') else {
            return Err(SettingsError::Parse {
                path: path.to_path_buf(),
                line: line_no,
                reason: "dòng phải dạng NAME=value".to_string(),
            });
        };
        let name = name.trim();
        if !is_env_name(name) {
            return Err(SettingsError::Parse {
                path: path.to_path_buf(),
                line: line_no,
                reason: format!("tên `{name}` phải dạng [A-Z_][A-Z0-9_]*"),
            });
        }
        if entries.iter().any(|(existing, _)| existing == name) {
            return Err(SettingsError::Parse {
                path: path.to_path_buf(),
                line: line_no,
                reason: format!("khoá `{name}` bị lặp"),
            });
        }
        validate_value(name, value)?;
        entries.push((name.to_string(), SecretString::from(value.to_string())));
    }
    Ok(SecretSnapshot { entries })
}

/// Ràng buộc giá trị: không rỗng, ≤ [`MAX_VALUE_BYTES`] byte, không ký tự điều khiển.
fn validate_value(name: &str, value: &str) -> Result<(), SettingsError> {
    if value.is_empty() {
        return Err(SettingsError::InvalidValue {
            name: name.to_string(),
            reason: "không được rỗng — muốn xoá thì dùng `secret unset`".to_string(),
        });
    }
    if value.len() > MAX_VALUE_BYTES {
        return Err(SettingsError::InvalidValue {
            name: name.to_string(),
            reason: format!("dài hơn {MAX_VALUE_BYTES} byte"),
        });
    }
    if let Some(position) = value.chars().position(char::is_control) {
        return Err(SettingsError::InvalidValue {
            name: name.to_string(),
            reason: format!("chứa ký tự điều khiển tại vị trí {position}"),
        });
    }
    Ok(())
}

/// Ghi body ra file: dòng comment đầu + mỗi entry một dòng.
fn serialize_entries(entries: &[(String, SecretString)]) -> String {
    let mut body = String::from("# Bean secrets — ghi bởi `bean secret set` / UI Cài đặt.\n");
    for (name, value) in entries {
        body.push_str(name);
        body.push('=');
        body.push_str(value.expose_secret());
        body.push('\n');
    }
    body
}

/// Ghi atomic: tmp tạo ngay với quyền `0600` → `fsync` → `rename` cùng thư mục.
fn write_atomic(path: &Path, body: &str) -> Result<(), SettingsError> {
    let tmp = sibling_path(path, "env.tmp");
    // Tmp còn sót từ lần ghi hỏng trước ⇒ xoá để `create_new` không thất bại.
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(SettingsError::Io {
                path: tmp.clone(),
                source: error,
            });
        }
    }
    let mut file = open_private(&tmp, true)?;
    file.write_all(body.as_bytes())
        .map_err(|error| SettingsError::Io {
            path: tmp.clone(),
            source: error,
        })?;
    file.sync_all().map_err(|error| SettingsError::Io {
        path: tmp.clone(),
        source: error,
    })?;
    drop(file);
    fs::rename(&tmp, path).map_err(|error| SettingsError::Io {
        path: path.to_path_buf(),
        source: error,
    })
}

/// Khoá file riêng (không bao giờ bị `rename`) cho read-modify-write của set/remove.
fn open_lock_file(target: &Path) -> Result<fd_lock::RwLock<File>, SettingsError> {
    let lock_path = sibling_path(target, "env.lock");
    let file = open_private(&lock_path, false)?;
    Ok(fd_lock::RwLock::new(file))
}

/// Đường dẫn cùng thư mục với `path`, thay phần mở rộng cuối.
fn sibling_path(path: &Path, extension: &str) -> PathBuf {
    let mut sibling = path.to_path_buf();
    sibling.set_extension(extension);
    sibling
}

/// Mở file với quyền `0600` **ngay lúc tạo**; `create_new` = chỉ tạo khi chưa có.
fn open_private(path: &Path, create_new: bool) -> Result<File, SettingsError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|error| SettingsError::Io {
        path: path.to_path_buf(),
        source: error,
    })
}

/// Thư mục chứa secret phải `0700` — siết ngay nếu rộng hơn (data.dir thường dùng chung).
#[cfg(unix)]
fn ensure_private_dir(dir: &Path) -> Result<(), SettingsError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(dir).map_err(|error| SettingsError::Io {
        path: dir.to_path_buf(),
        source: error,
    })?;
    if metadata.permissions().mode() & 0o077 != 0 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
            SettingsError::Io {
                path: dir.to_path_buf(),
                source: error,
            }
        })?;
        tracing::warn!(
            dir = %dir.display(),
            "thư mục chứa secret có quyền rộng — đã siết về 0700"
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_dir(_dir: &Path) -> Result<(), SettingsError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::slot::SecretSlotRegistry;

    fn registry(names: &[&str]) -> SecretSlotRegistry {
        SecretSlotRegistry::from_names(names)
    }

    fn secret(value: &str) -> SecretString {
        SecretString::from(value.to_string())
    }

    #[cfg(unix)]
    fn file_mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn fresh_set_creates_file_0600_and_missing_dir_0700() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let store = SecretStore::new(data.join("secrets.env"));
        let registry = registry(&["BEAN_TEST_KEY"]);
        store
            .set(&registry, "BEAN_TEST_KEY", secret("sk-1"))
            .unwrap();

        assert_eq!(
            store
                .snapshot()
                .unwrap()
                .get("BEAN_TEST_KEY")
                .map(|v| v.expose_secret().to_string()),
            Some("sk-1".to_string())
        );
        #[cfg(unix)]
        {
            assert_eq!(
                file_mode(&data.join("secrets.env")),
                0o600,
                "file phải 0600 ngay lúc tạo"
            );
            assert_eq!(file_mode(&data), 0o700, "thư mục phải 0700");
        }
    }

    #[test]
    fn second_set_preserves_first_entry() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.env"));
        let registry = registry(&["BEAN_TEST_A", "BEAN_TEST_B"]);
        store.set(&registry, "BEAN_TEST_A", secret("a")).unwrap();
        store.set(&registry, "BEAN_TEST_B", secret("b")).unwrap();
        let snapshot = store.snapshot().unwrap();
        assert_eq!(
            snapshot.names().collect::<Vec<_>>(),
            vec!["BEAN_TEST_A", "BEAN_TEST_B"]
        );
    }

    #[test]
    fn set_overwrites_value_without_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.env"));
        let registry = registry(&["BEAN_TEST_A"]);
        store.set(&registry, "BEAN_TEST_A", secret("old")).unwrap();
        store.set(&registry, "BEAN_TEST_A", secret("new")).unwrap();
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.names().collect::<Vec<_>>(), vec!["BEAN_TEST_A"]);
        assert_eq!(
            snapshot
                .get("BEAN_TEST_A")
                .map(|v| v.expose_secret().to_string()),
            Some("new".to_string())
        );
    }

    #[test]
    fn remove_reports_presence() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.env"));
        let registry = registry(&["BEAN_TEST_A"]);
        store.set(&registry, "BEAN_TEST_A", secret("a")).unwrap();
        assert!(store.remove(&registry, "BEAN_TEST_A").unwrap());
        assert!(!store.remove(&registry, "BEAN_TEST_A").unwrap());
        assert!(store.snapshot().unwrap().is_empty());
    }

    #[test]
    fn set_rejects_names_outside_declared_slots() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.env"));
        let registry = registry(&["BEAN_TEST_A"]);
        for name in ["PATH", "LD_PRELOAD", "HTTPS_PROXY", "path", "1BAD"] {
            let error = store
                .set(&registry, name, secret("x"))
                .expect_err("tên ngoài slot phải bị từ chối");
            assert!(
                matches!(error, SettingsError::UnknownSlot { .. }),
                "{error}"
            );
        }
        assert!(store.snapshot().unwrap().is_empty(), "không được ghi gì");
    }

    #[test]
    fn set_rejects_bad_values() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.env"));
        let registry = registry(&["BEAN_TEST_A"]);
        let too_long = String::from_utf8(vec![b'a'; MAX_VALUE_BYTES + 1]).unwrap();
        for bad in ["", "line1\nline2", "nul\u{0}char", too_long.as_str()] {
            let error = store
                .set(&registry, "BEAN_TEST_A", secret(bad))
                .expect_err("giá trị sai phải bị từ chối");
            assert!(
                matches!(error, SettingsError::InvalidValue { .. }),
                "{error}"
            );
        }
        assert!(store.snapshot().unwrap().is_empty());
    }

    #[test]
    fn value_at_limit_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.env"));
        let registry = registry(&["BEAN_TEST_A"]);
        let at_limit = String::from_utf8(vec![b'a'; MAX_VALUE_BYTES]).unwrap();
        store
            .set(&registry, "BEAN_TEST_A", secret(&at_limit))
            .unwrap();
        assert!(store.snapshot().unwrap().contains("BEAN_TEST_A"));
    }

    #[test]
    fn snapshot_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.env"));
        assert!(store.snapshot().unwrap().is_empty());
    }

    #[test]
    fn corrupt_file_fails_and_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        fs::write(&path, "không phải định dạng").unwrap();
        let store = SecretStore::new(path.clone());
        assert!(store.snapshot().is_err());
        let registry = registry(&["BEAN_TEST_A"]);
        assert!(store.set(&registry, "BEAN_TEST_A", secret("x")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "không phải định dạng");
    }

    #[test]
    fn duplicate_name_in_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        fs::write(&path, "BEAN_TEST_A=one\nBEAN_TEST_A=two\n").unwrap();
        let error = SecretStore::new(path.clone())
            .snapshot()
            .expect_err("khoá lặp phải bị từ chối");
        assert!(
            matches!(error, SettingsError::Parse { line: 2, .. }),
            "{error}"
        );
    }

    #[test]
    fn comments_blank_lines_and_crlf_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        fs::write(
            &path,
            "# ghi chú\r\n\r\nBEAN_TEST_A=one\r\n  # comment lệch\r\n",
        )
        .unwrap();
        let snapshot = SecretStore::new(path).snapshot().unwrap();
        assert_eq!(
            snapshot
                .get("BEAN_TEST_A")
                .map(|v| v.expose_secret().to_string()),
            Some("one".to_string())
        );
    }

    #[test]
    fn interleaved_writes_from_two_stores_keep_everything() {
        let dir = tempfile::tempdir().unwrap();
        let store_a = SecretStore::new(dir.path().join("secrets.env"));
        let store_b = store_a.clone();
        let registry = registry(&["BEAN_TEST_A", "BEAN_TEST_B"]);
        // Mô phỏng CLI + server ghi đè nhau: đọc lại file dưới khoá nên không mất entry.
        store_a.set(&registry, "BEAN_TEST_A", secret("a")).unwrap();
        store_b.set(&registry, "BEAN_TEST_B", secret("b")).unwrap();
        store_a.set(&registry, "BEAN_TEST_A", secret("a2")).unwrap();
        let snapshot = store_a.snapshot().unwrap();
        assert_eq!(
            snapshot.names().collect::<Vec<_>>(),
            vec!["BEAN_TEST_A", "BEAN_TEST_B"]
        );
        assert_eq!(
            snapshot
                .get("BEAN_TEST_A")
                .map(|v| v.expose_secret().to_string()),
            Some("a2".to_string())
        );
    }

    #[test]
    fn concurrent_writers_do_not_lose_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(SecretStore::new(dir.path().join("secrets.env")));
        let names: Vec<String> = (0..8)
            .map(|index| format!("BEAN_TEST_KEY_{index}"))
            .collect();
        let registry = std::sync::Arc::new(registry(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
        let mut handles = Vec::new();
        for thread_index in 0..2 {
            let store = std::sync::Arc::clone(&store);
            let registry = std::sync::Arc::clone(&registry);
            let names = names.clone();
            handles.push(std::thread::spawn(move || {
                for (index, name) in names.iter().enumerate() {
                    if (index % 2) == thread_index {
                        store.set(&registry, name, secret(name)).unwrap();
                    }
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        let snapshot = store.snapshot().unwrap();
        for name in &names {
            assert!(
                snapshot.contains(name),
                "mất entry `{name}` khi ghi đồng thời"
            );
        }
    }

    #[test]
    fn debug_of_snapshot_never_shows_value() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.env"));
        let registry = registry(&["BEAN_TEST_A"]);
        store
            .set(&registry, "BEAN_TEST_A", secret("sk-never-leak-please"))
            .unwrap();
        let rendered = format!("{:?}", store.snapshot().unwrap());
        assert!(!rendered.contains("sk-never-leak-please"), "{rendered}");
    }
}
