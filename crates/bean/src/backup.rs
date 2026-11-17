//! `bean backup` — snapshot nhất quán của `data.dir` thành một file `.tar.gz`.
//!
//! # Vì sao là lệnh của Bean thay vì `cp`
//!
//! `bean.db` chạy `journal_mode=WAL` và tiến trình serve **đang ghi** vào nó; copy
//! từng file (db + wal + shm) trong lúc chạy có thể cho bản chụp lẫn nửa vời —
//! đúng lớp lỗi mà restore lúc cần nhất lại hỏng. Backup API của SQLite
//! (`Connection::backup`) chụp qua một read transaction nên bản chụp luôn nhất
//! quán ngay cả khi instance đang phục vụ chat, không cần dừng Bean.
//!
//! # Phạm vi đóng gói
//!
//! - `bean.db` → bản chụp nhất quán (không kèm `-wal`/`-shm`).
//! - Mọi file thường cấp một của `data.dir` (`auth.toml`, `llm-state.json`,
//!   `voice-state.json`, `settings-state.json`, `secrets.env`…) — trừ file
//!   khoá (`*.lock`) và `--no-secrets` bỏ `secrets.env` khi gửi backup ra ngoài.
//! - Toàn bộ thư mục con (`audit/`, `checkpoints/`, `skills/`…).
//!
//! Quyền file (`0600` cho auth/secrets) được giữ trong tar vì `append_file` lấy
//! metadata thật; restore chỉ cần giải nén vào thư mục mới và chạy lại Bean.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use bean_types::Config;
use chrono::Utc;
use flate2::Compression;
use flate2::write::GzEncoder;
use rusqlite::backup::Progress;
use tar::Builder;

/// Tham số của `bean backup`.
#[derive(Debug, Clone)]
pub struct BackupArgs {
    /// File `.tar.gz` đầu ra; `None` → `bean-backup-<timestamp>.tar.gz` ở CWD.
    pub dest: Option<PathBuf>,
    /// Không đóng gói `secrets.env` (backup mang ra khỏi máy).
    pub no_secrets: bool,
}

/// Tên file DB của store (cùng nguồn sự thật với `chat::store_path`).
const DB_FILE: &str = "bean.db";
/// Sản phẩm phụ của WAL không cần vào backup — bản chụp đã chứa dữ liệu của chúng.
const DB_SIDECARS: [&str; 2] = ["bean.db-wal", "bean.db-shm"];
/// File khoá do tiến trình tạo, vô nghĩa khi restore.
const LOCK_SUFFIX: &str = ".lock";
const SECRETS_FILE: &str = "secrets.env";

/// Chạy lệnh `backup` (agents.md mục 18 — vận hành).
///
/// # Errors
/// Lỗi khi nạp cấu hình, mở DB, chụp snapshot, kiểm tra integrity hay ghi tar.gz.
pub fn run(args: &BackupArgs, config_path: Option<&Path>) -> Result<()> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let data_dir = crate::chat::expand_tilde(&config.data.dir);
    if !data_dir.is_dir() {
        bail!("data.dir không tồn tại: {}", data_dir.display());
    }
    let dest = args.dest.clone().unwrap_or_else(default_dest);
    create_backup(&data_dir, &dest, !args.no_secrets)
        .with_context(|| format!("backup vào {}", dest.display()))?;
    println!(
        "Đã backup {} → {} ({} byte).",
        data_dir.display(),
        dest.display(),
        fs::metadata(&dest).map_or(0, |m| m.len())
    );
    Ok(())
}

/// Tạo backup: chụp DB nhất quán + đóng gói phần còn lại của `data.dir`.
///
/// `include_secrets = false` bỏ `secrets.env` — dùng khi archive đi ra ngoài máy.
fn create_backup(data_dir: &Path, dest: &Path, include_secrets: bool) -> Result<()> {
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("tạo {}", parent.display()))?;
    }

    // 1. Chụp DB qua backup API — nguồn có thể đang được serve ghi đồng thời.
    let snapshot =
        snapshot_database(&data_dir.join(DB_FILE)).with_context(|| format!("chụp {}", DB_FILE))?;

    let result = (|| {
        let output = fs::File::create(dest).with_context(|| format!("tạo {}", dest.display()))?;
        let encoder = GzEncoder::new(output, Compression::default());
        let mut tar = Builder::new(encoder);

        // `append_file` giữ nguyên metadata (HeaderMode::Complete) nên quyền
        // `0600` của auth/secrets sống sót qua tar.
        let mut db_file = fs::File::open(&snapshot)
            .with_context(|| format!("mở bản chụp {}", snapshot.display()))?;
        tar.append_file(DB_FILE, &mut db_file)
            .with_context(|| format!("đóng gói {}", DB_FILE))?;

        for entry in
            fs::read_dir(data_dir).with_context(|| format!("đọc {}", data_dir.display()))?
        {
            let entry = entry.context("đọc entry data.dir")?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("đọc loại {}", name))?;

            if file_type.is_dir() {
                tar.append_dir_all(&name, &path)
                    .with_context(|| format!("đóng gói thư mục {name}"))?;
                continue;
            }
            if DB_SIDECARS.contains(&name.as_str())
                || name.ends_with(LOCK_SUFFIX)
                || name == DB_FILE
                || (!include_secrets && name == SECRETS_FILE)
            {
                continue;
            }
            let mut file = fs::File::open(&path).with_context(|| format!("mở {name}"))?;
            tar.append_file(&name, &mut file)
                .with_context(|| format!("đóng gói {name}"))?;
        }

        let encoder = tar.into_inner().context("chốt archive tar")?;
        let file = encoder.finish().context("chốt gzip")?;
        file.sync_all().context("flush archive")?;
        Ok(())
    })();
    let _ = fs::remove_file(&snapshot);
    result
}

/// Bộ đếm để hai lần backup trong cùng tiến trình không bao giờ trùng file tạm
/// (pid + nanos vẫn có thể trùng nếu hai test song song khởi động cùng nhịp).
static SNAPSHOT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Chụp `bean.db` vào file tạm và tự kiểm tra integrity trước khi đóng gói.
fn snapshot_database(db_path: &Path) -> Result<PathBuf> {
    let snapshot = std::env::temp_dir().join(format!(
        "bean-backup-db-{}-{}-{}.db",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        SNAPSHOT_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let src =
        rusqlite::Connection::open(db_path).with_context(|| format!("mở {}", db_path.display()))?;
    src.backup("main", &snapshot, None::<fn(Progress)>)
        .context("chụp SQLite (backup API)")?;

    let copy = rusqlite::Connection::open(&snapshot).context("mở bản chụp")?;
    let status: String = copy
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .context("PRAGMA integrity_check")?;
    if status != "ok" {
        bail!("bản chụp không đạt integrity_check: {status}");
    }
    Ok(snapshot)
}

/// `bean-backup-20261117T033000Z.tar.gz` ở thư mục hiện tại.
fn default_dest() -> PathBuf {
    PathBuf::from(format!(
        "bean-backup-{}.tar.gz",
        Utc::now().format("%Y%m%dT%H%M%SZ")
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::io::Read;

    use super::*;

    /// Dựng `data.dir` giả: DB có một hàng, auth 0600, audit một file, WAL rác.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        fs::create_dir_all(data.join("audit")).unwrap();
        let db = rusqlite::Connection::open(data.join(DB_FILE)).unwrap();
        db.execute_batch("CREATE TABLE t(v TEXT); INSERT INTO t VALUES('giữ tôi');")
            .unwrap();
        drop(db);
        fs::write(data.join(DB_SIDECARS[0]), "wal rác").unwrap();
        let auth = data.join("auth.toml");
        fs::write(&auth, b"hash = 'x'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&auth, fs::Permissions::from_mode(0o600)).unwrap();
        }
        fs::write(data.join("audit").join("a.jsonl"), b"{\"tool\":\"x\"}\n").unwrap();
        (temp, data)
    }

    fn read_entry(archive_path: &Path, name: &str) -> Option<Vec<u8>> {
        let file = fs::File::open(archive_path).unwrap();
        let gz = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz);
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().to_path_buf();
            if path.to_string_lossy() == name {
                let mut buffer = Vec::new();
                entry.read_to_end(&mut buffer).unwrap();
                return Some(buffer);
            }
        }
        None
    }

    #[test]
    fn backup_chua_db_nhat_quan_auth_audit_va_bo_sidecar() {
        let (temp, data) = fixture();
        let dest = temp.path().join("out").join("backup.tar.gz");
        create_backup(&data, &dest, true).unwrap();

        let db_bytes = read_entry(&dest, DB_FILE).expect("bean.db trong archive");
        let restored = tempfile::tempdir().unwrap();
        let db_path = restored.path().join("restored.db");
        fs::write(&db_path, db_bytes).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let value: String = conn
            .query_row("SELECT v FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "giữ tôi");

        assert_eq!(read_entry(&dest, "auth.toml").unwrap(), b"hash = 'x'\n");
        assert_eq!(
            read_entry(&dest, "audit/a.jsonl").unwrap(),
            b"{\"tool\":\"x\"}\n"
        );
        assert!(
            read_entry(&dest, DB_SIDECARS[0]).is_none(),
            "WAL không vào backup"
        );
    }

    #[test]
    fn backup_no_secrets_bo_secrets_env() {
        let (temp, data) = fixture();
        fs::write(data.join(SECRETS_FILE), b"KEY=x\n").unwrap();
        let dest = temp.path().join("backup.tar.gz");
        create_backup(&data, &dest, true).unwrap();
        assert_eq!(read_entry(&dest, SECRETS_FILE).unwrap(), b"KEY=x\n");

        let dest_no_secrets = temp.path().join("backup-no-secrets.tar.gz");
        create_backup(&data, &dest_no_secrets, false).unwrap();
        assert!(read_entry(&dest_no_secrets, SECRETS_FILE).is_none());
        // DB và phần còn lại vẫn đủ.
        assert!(read_entry(&dest_no_secrets, DB_FILE).is_some());
    }

    #[test]
    fn backup_that_bai_khi_data_dir_khong_ton_tai() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("khong-ton-tai");
        let result = create_backup(&missing, &temp.path().join("b.tar.gz"), true);
        assert!(
            result.is_err(),
            "data.dir rỗng phải báo lỗi, không tạo archive rác"
        );
    }
}
