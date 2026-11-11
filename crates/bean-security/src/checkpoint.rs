//! Snapshot workspace cho một lượt (D26) — cài đặt
//! [`bean_tools::WorkspaceCheckpoint`] cho tool `rollback_workspace`.
//!
//! # Ranh giới bảo mật
//!
//! Toàn bộ đường dẫn ở đây đến từ **cấu hình vận hành** (`[data] dir`,
//! `[agent] workspace`) — không bao giờ từ model. Khác với tool file (đầu vào
//! không tin cậy, phải qua jail cap-std), module này là hạ tầng như audit log:
//! dùng `std::fs` trực tiếp, chạy trong `spawn_blocking` để không chặn executor.
//! Snapshot nằm trong `data.dir/checkpoints/` — **ngoài** jail workspace nên
//! model không đọc/xoá được dấu vết của chính mình.
//!
//! # Độ bền
//!
//! Copy vào `<id>.partial` rồi rename: run bị huỷ giữa chừng không bao giờ để
//! lại checkpoint dở dang bị nhận nhầm là hoàn chỉnh. Khôi phục ghi đè từ
//! snapshot **trước** rồi mới xoá file thừa — gián đoạn giữa chừng để lại
//! workspace nửa vằng vặt nhưng snapshot còn nguyên, chạy lại được.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use bean_tools::ToolError;
use bean_tools::checkpoint::{ChangeList, RestoreReport, WorkspaceCheckpoint};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Độ lệch mtime (giây) coi như "không đổi" khi so sánh file giữa snapshot và
/// hiện tại — heuristic cho thẻ xác nhận; khôi phục luôn ghi đè nội dung thật.
const MTIME_TOLERANCE_SECS: u64 = 2;

/// Lỗi của việc chụp checkpoint.
#[derive(Debug, Error)]
pub enum CheckpointError {
    /// Workspace vượt trần dung lượng — không chụp, tool ghi bị từ chối.
    #[error(
        "workspace {size} byte vượt trần checkpoint {max} byte — tăng \
         [security.checkpoint].max_workspace_mb hoặc tắt [security.checkpoint].enabled"
    )]
    TooLarge {
        /// Dung lượng đo được của workspace (byte).
        size: u64,
        /// Trần cho phép (byte).
        max: u64,
    },
    /// Lỗi hệ thống file (kèm ngữ cảnh tiếng Việt).
    #[error("lỗi hệ thống file: {context}: {source}")]
    Io {
        /// Mô tả thao tác bị lỗi.
        context: String,
        /// Lỗi gốc.
        #[source]
        source: std::io::Error,
    },
}

/// Kho checkpoint của tiến trình — một instance dùng chung mọi lượt (DIP: core
/// cầm `Arc<CheckpointStore>`, không biết chi tiết file).
#[derive(Debug)]
pub struct CheckpointStore {
    /// Thư mục gốc chứa mọi snapshot (`data.dir/checkpoints/`).
    root: PathBuf,
    /// Số checkpoint giữ lại khi prune.
    keep: usize,
    /// Trần dung lượng workspace được chụp (byte).
    max_workspace_bytes: u64,
}

impl CheckpointStore {
    /// Tạo kho tại `root` (thư mục tự tạo lúc chụp, không tạo ở đây để test
    /// dựng được store trỏ vào thư mục chưa tồn tại).
    #[must_use]
    pub fn new(root: PathBuf, config: &bean_types::config::CheckpointConfig) -> Self {
        Self {
            root,
            keep: usize::try_from(config.keep.max(1)).unwrap_or(usize::MAX),
            max_workspace_bytes: config.max_workspace_mb.saturating_mul(1024 * 1024),
        }
    }

    /// Chụp workspace vào một checkpoint mới.
    ///
    /// # Errors
    /// [`CheckpointError::TooLarge`] khi workspace vượt trần; [`CheckpointError::Io`]
    /// khi không đọc/ghi được. Lỗi copy luôn dọn thư mục `.partial`.
    pub async fn capture(
        &self,
        workspace_root: &Path,
    ) -> Result<Arc<RunCheckpoint>, CheckpointError> {
        // Định danh sinh TRƯỚC task blocking: hai capture đồng thời trong cùng
        // tiến trình không đụng nhau (mỗi lượt một thư mục riêng).
        let id = new_checkpoint_id();
        let root = self.root.clone();
        let max = self.max_workspace_bytes;
        let workspace = workspace_root.to_path_buf();
        let partial = root.join(format!("{id}.partial"));
        let keep = self.keep;
        let root_for_prune = root.clone();
        let id_in_task = id.clone();
        let capture: Result<(), CheckpointError> = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&root).map_err(|source| CheckpointError::Io {
                context: format!("tạo thư mục checkpoint {}", root.display()),
                source,
            })?;
            cleanup_stale_partials(&root);
            let size = measure(&workspace)
                .map_err(|(context, source)| CheckpointError::Io { context, source })?;
            if size > max {
                return Err(CheckpointError::TooLarge { size, max });
            }
            if partial.exists() {
                std::fs::remove_dir_all(&partial).map_err(|source| CheckpointError::Io {
                    context: format!("dọn thư mục tạm {}", partial.display()),
                    source,
                })?;
            }
            copy_tree(&workspace, &partial).map_err(|(context, source)| {
                // Copy hỏng ⇒ dọn partial, không để lại rác.
                let _ = std::fs::remove_dir_all(&partial);
                CheckpointError::Io { context, source }
            })?;
            let final_dir = root.join(&id_in_task);
            std::fs::rename(&partial, &final_dir).map_err(|source| {
                let _ = std::fs::remove_dir_all(&partial);
                CheckpointError::Io {
                    context: format!(
                        "đổi tên {} thành {}",
                        partial.display(),
                        final_dir.display()
                    ),
                    source,
                }
            })?;
            Ok(())
        })
        .await
        .map_err(|error| CheckpointError::Io {
            context: "task chụp checkpoint bị hủy".into(),
            source: std::io::Error::other(error),
        })?;
        capture?;
        prune_checkpoints(&root_for_prune, keep.max(1));
        let snapshot = root_for_prune.join(&id);
        let created_at = chrono::Utc::now().to_rfc3339();
        Ok(Arc::new(RunCheckpoint {
            id,
            snapshot,
            created_at,
        }))
    }
}

/// Hậu tố thư mục copy tạm trước khi rename.
const CHECKPOINT_PARTIAL_SUFFIX: &str = ".partial";

/// Checkpoint của một lượt — snapshot đường dẫn + thời điểm chụp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCheckpoint {
    /// Định danh (`<utc timestamp>-<8 hex>`), đồng thời là tên thư mục snapshot.
    id: String,
    /// Đường tuyệt đối của snapshot.
    snapshot: PathBuf,
    /// Thời điểm chụp (RFC3339 UTC).
    created_at: String,
}

#[async_trait]
impl WorkspaceCheckpoint for RunCheckpoint {
    fn label(&self) -> String {
        format!("checkpoint {} (chụp lúc {})", self.id, self.created_at)
    }

    async fn changes(&self, workspace_root: &Path) -> Result<ChangeList, ToolError> {
        let snapshot = self.snapshot.clone();
        let workspace = workspace_root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let snap = scan(&snapshot).map_err(|(context, source)| {
                ToolError::Io(format!(
                    "đọc snapshot {}: {context}: {source}",
                    snapshot.display()
                ))
            })?;
            let now = scan(&workspace).map_err(|(context, source)| {
                ToolError::Io(format!(
                    "đọc workspace {}: {context}: {source}",
                    workspace.display()
                ))
            })?;
            let mut changes = ChangeList::default();
            for (path, meta) in &now {
                match snap.get(path) {
                    None => changes.added.push((*path).clone()),
                    Some(old) => {
                        if old.0 != meta.0 || !mtime_close(old.1, meta.1) {
                            changes.modified.push((*path).clone());
                        }
                    }
                }
            }
            for path in snap.keys() {
                if !now.contains_key(path) {
                    changes.removed.push(path.clone());
                }
            }
            changes.added.sort();
            changes.modified.sort();
            changes.removed.sort();
            Ok(changes)
        })
        .await
        .map_err(|error| ToolError::Io(format!("task so sánh checkpoint bị hủy: {error}")))?
    }

    async fn restore(&self, workspace_root: &Path) -> Result<RestoreReport, ToolError> {
        let snapshot = self.snapshot.clone();
        let workspace = workspace_root.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let snap = scan(&snapshot).map_err(|(context, source)| {
                ToolError::Io(format!(
                    "đọc snapshot {}: {context}: {source}",
                    snapshot.display()
                ))
            })?;
            let now = scan(&workspace).map_err(|(context, source)| {
                ToolError::Io(format!(
                    "đọc workspace {}: {context}: {source}",
                    workspace.display()
                ))
            })?;
            let mut report = RestoreReport::default();
            // Bước 1 — ghi đè từ snapshot TRƯỚC: hỏng giữa chừng vẫn còn file cũ
            // trên đĩa, chạy lại được (nhìn tổng quan trong doc header).
            for path in snap.keys() {
                let from = snapshot.join(path);
                let target = workspace.join(path);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        ToolError::Io(format!("tạo thư mục cha cho {}: {error}", target.display()))
                    })?;
                }
                std::fs::copy(&from, &target).map_err(|error| {
                    ToolError::Io(format!(
                        "khôi phục {} vào {}: {error}",
                        from.display(),
                        target.display()
                    ))
                })?;
                report.restored += 1;
            }
            // Bước 2 — xoá file sinh sau checkpoint, rồi dọn thư mục rỗng.
            for path in now.keys() {
                if !snap.contains_key(path) {
                    std::fs::remove_file(workspace.join(path)).map_err(|error| {
                        ToolError::Io(format!(
                            "xoá file thừa {}: {error}",
                            workspace.join(path).display()
                        ))
                    })?;
                    report.removed += 1;
                }
            }
            remove_empty_dirs(&workspace);
            Ok(report)
        })
        .await
        .map_err(|error| ToolError::Io(format!("task khôi phục checkpoint bị hủy: {error}")))?
    }
}

/// Sinh định danh checkpoint: UTC timestamp + 8 hex ngẫu nhiên (CSPRNG của OS).
fn new_checkpoint_id() -> String {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let mut bytes = [0_u8; 4];
    getrandom::fill(&mut bytes).unwrap_or_else(|_| {
        // Không xảy ra trên Linux/Windows/macOS (getrandom đọc /dev/urandom hay
        // BCryptGenRandom); rơi về pid để vẫn phân biệt được hai checkpoint.
        bytes = std::process::id().to_le_bytes();
    });
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{stamp}-{hex}")
}

/// Đo tổng dung lượng **file** của cây (symlink không đi qua, tính 0).
fn measure(root: &Path) -> Result<u64, (String, std::io::Error)> {
    let meta = std::fs::symlink_metadata(root)
        .map_err(|source| (format!("đọc metadata {}", root.display()), source))?;
    if meta.file_type().is_symlink() {
        return Ok(0);
    }
    if !meta.is_dir() {
        return Ok(meta.len());
    }
    let entries = std::fs::read_dir(root)
        .map_err(|source| (format!("đọc thư mục {}", root.display()), source))?;
    let mut total = 0_u64;
    for entry in entries {
        let entry =
            entry.map_err(|source| (format!("đọc entry trong {}", root.display()), source))?;
        total += measure(&entry.path()).map_err(|(context, source)| {
            (format!("{}: {context}", entry.path().display()), source)
        })?;
    }
    Ok(total)
}

/// Duyệt đệ quy trả map `đường dẫn tương đối → (kích thước, mtime)`, chỉ file,
/// symlink không đi qua (chống vòng lặp).
fn scan(
    root: &Path,
) -> Result<std::collections::BTreeMap<String, (u64, SystemTime)>, (String, std::io::Error)> {
    fn walk(
        dir: &Path,
        rel: &Path,
        out: &mut std::collections::BTreeMap<String, (u64, SystemTime)>,
    ) -> Result<(), (String, std::io::Error)> {
        let entries = std::fs::read_dir(dir)
            .map_err(|source| (format!("đọc thư mục {}", dir.display()), source))?;
        for entry in entries {
            let entry =
                entry.map_err(|source| (format!("đọc entry trong {}", dir.display()), source))?;
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|source| (format!("đọc metadata {}", path.display()), source))?;
            let child_rel = rel.join(entry.file_name());
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                walk(&path, &child_rel, out)?;
            } else if meta.is_file() {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                out.insert(
                    child_rel.to_string_lossy().replace('\\', "/"),
                    (meta.len(), mtime),
                );
            }
        }
        Ok(())
    }
    let mut out = std::collections::BTreeMap::new();
    walk(root, Path::new(""), &mut out)?;
    Ok(out)
}

/// Copy đệ quy cây `src` sang `dst` (tạo `dst`); symlink được tái tạo nguyên
/// xi trên Unix (Windows bỏ qua — workspace cá nhân hầu như không có).
#[allow(clippy::similar_names)]
fn copy_tree(src: &Path, dst: &Path) -> Result<(), (String, std::io::Error)> {
    std::fs::create_dir_all(dst).map_err(|source| (format!("tạo {}", dst.display()), source))?;
    let entries =
        std::fs::read_dir(src).map_err(|source| (format!("đọc {}", src.display()), source))?;
    for entry in entries {
        let entry = entry.map_err(|source| (format!("đọc entry {}", src.display()), source))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = std::fs::symlink_metadata(&from)
            .map_err(|source| (format!("đọc metadata {}", from.display()), source))?;
        if meta.file_type().is_symlink() {
            #[cfg(unix)]
            {
                let target = std::fs::read_link(&from)
                    .map_err(|source| (format!("đọc symlink {}", from.display()), source))?;
                std::os::unix::fs::symlink(&target, &to)
                    .map_err(|source| (format!("tạo symlink {}", to.display()), source))?;
            }
        } else if meta.is_dir() {
            copy_tree(&from, &to)?;
        } else if meta.is_file() {
            std::fs::copy(&from, &to).map_err(|source| {
                (
                    format!("copy {} → {}", from.display(), to.display()),
                    source,
                )
            })?;
        }
    }
    Ok(())
}

/// Xoá các `.partial*` sót lại (từ crash trước đó) — capture là writer duy nhất
/// và rename gần như tức thời nên mọi partial cũ đều là rác.
fn cleanup_stale_partials(root: &Path) {
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name
                .to_string_lossy()
                .starts_with(CHECKPOINT_PARTIAL_SUFFIX)
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// Giữ `keep` checkpoint mới nhất (tên có prefix UTC nên sort tên = sort thời gian),
/// xoá phần còn lại.
fn prune_checkpoints(root: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with(CHECKPOINT_PARTIAL_SUFFIX))
        .collect();
    names.sort();
    if names.len() <= keep {
        return;
    }
    let excess = names.len() - keep;
    for name in names.into_iter().take(excess) {
        let _ = std::fs::remove_dir_all(root.join(name));
    }
}

/// `true` khi hai mtime lệch không quá [`MTIME_TOLERANCE_SECS`].
fn mtime_close(a: SystemTime, b: SystemTime) -> bool {
    match (
        a.duration_since(SystemTime::UNIX_EPOCH),
        b.duration_since(SystemTime::UNIX_EPOCH),
    ) {
        (Ok(a), Ok(b)) => a.abs_diff(b) <= std::time::Duration::from_secs(MTIME_TOLERANCE_SECS),
        _ => true,
    }
}

/// Xoá thư mục rỗng (dọn các thư mục sinh sau checkpoint rồi bị rút hết file).
fn remove_empty_dirs(root: &Path) {
    fn walk(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut empty = true;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                empty &= walk(&path);
            } else {
                empty = false;
            }
        }
        if empty {
            let _ = std::fs::remove_dir(dir);
        }
        empty
    }
    walk(root);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn store_in(dir: &Path, keep: usize) -> CheckpointStore {
        CheckpointStore::new(
            dir.join("checkpoints"),
            &bean_types::config::CheckpointConfig {
                enabled: true,
                keep: keep as u32,
                max_workspace_mb: 200,
            },
        )
    }

    /// Dựng workspace với danh sách `rel → nội dung` (tự tạo thư mục cha).
    fn build_tree(root: &Path, files: &[(&str, &str)]) {
        for (rel, content) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
    }

    /// Đọc toàn bộ cây thành map `rel → nội dung` để so sánh byte-for-byte.
    fn read_tree(root: &Path) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        for (rel, _) in scan(root).unwrap() {
            let content = std::fs::read_to_string(root.join(&rel)).unwrap();
            out.insert(rel, content);
        }
        out
    }

    #[tokio::test]
    async fn roundtrip_khoi_phuc_dung_noi_dung_file() {
        let temp = tempfile::tempdir().unwrap();
        let ws = temp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        build_tree(
            &ws,
            &[
                ("note.txt", "gốc"),
                ("sub/deep/ghi-chu.md", "nội dung tiếng Việt + emoji 🌟"),
            ],
        );
        let store = store_in(temp.path(), 3);
        let checkpoint = store.capture(&ws).await.unwrap();

        // Model "làm hỏng": sửa 1, xoá 1, thêm 1.
        std::fs::write(ws.join("note.txt"), "đã ghi sai").unwrap();
        std::fs::remove_file(ws.join("sub/deep/ghi-chu.md")).unwrap();
        std::fs::write(ws.join("moi.txt"), "file sinh sau checkpoint").unwrap();

        let changes = checkpoint.changes(&ws).await.unwrap();
        assert_eq!(changes.modified, vec!["note.txt".to_string()]);
        assert_eq!(changes.removed, vec!["sub/deep/ghi-chu.md".to_string()]);
        assert_eq!(changes.added, vec!["moi.txt".to_string()]);

        let report = checkpoint.restore(&ws).await.unwrap();
        assert_eq!(report.restored, 2);
        assert_eq!(report.removed, 1);
        assert_eq!(
            read_tree(&ws),
            std::collections::BTreeMap::from([
                ("note.txt".into(), "gốc".into()),
                (
                    "sub/deep/ghi-chu.md".into(),
                    "nội dung tiếng Việt + emoji 🌟".into()
                ),
            ])
        );
        // Sau khôi phục, changes phải rỗng.
        assert!(checkpoint.changes(&ws).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn workspace_qua_tran_thi_tu_choi_khoi_tao_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let ws = temp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let store = CheckpointStore::new(
            temp.path().join("checkpoints"),
            &bean_types::config::CheckpointConfig {
                enabled: true,
                keep: 3,
                max_workspace_mb: 0, // trần 0 MB — luôn từ chối
            },
        );
        std::fs::write(ws.join("a.txt"), "x").unwrap();
        let error = store.capture(&ws).await.unwrap_err();
        assert!(matches!(error, CheckpointError::TooLarge { .. }), "{error}");
        assert!(!temp.path().join("checkpoints").join("a").exists());
    }

    #[tokio::test]
    async fn prune_giu_dung_so_checkpoint_moi_nhat() {
        let temp = tempfile::tempdir().unwrap();
        let ws = temp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("a.txt"), "x").unwrap();
        let store = store_in(temp.path(), 2);
        for _ in 0..4 {
            store.capture(&ws).await.unwrap();
            // Tên checkpoint có giây UTC — hai lần trong cùng một giây trùng tên?
            // id có 8 hex ngẫu nhiên nên không trùng, nhưng thứ tự sort theo tên
            // trong cùng giây là theo hex — prune theo tên vẫn giữ đúng số lượng.
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        let entries: Vec<_> = std::fs::read_dir(temp.path().join("checkpoints"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| !entry.file_name().to_string_lossy().contains(".partial"))
            .collect();
        assert_eq!(entries.len(), 2, "phải prune còn 2 checkpoint");
    }

    #[tokio::test]
    async fn capture_that_khong_de_lai_partial() {
        let temp = tempfile::tempdir().unwrap();
        let ws = temp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        // File thường + thư mục không đọc được? Hard mô phỏng lỗi IO trên Linux
        // với user thường — thay vào đó chỉ kiểm chứng luồng thành công không
        // để lại .partial nào.
        std::fs::write(ws.join("a.txt"), "x").unwrap();
        let store = store_in(temp.path(), 3);
        store.capture(&ws).await.unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(temp.path().join("checkpoints"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".partial"))
            .collect();
        assert!(leftovers.is_empty());
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(16))]

        #[test]
        fn roundtrip_cay_ngau_nhien_khoi_phuc_byte_for_byte(
            n in 1_usize..12,
            seeds in proptest::prelude::prop::collection::vec(
                ("[a-zàáạăâđèéêôơư0-9_]{1,12}", "[a-z0-9 🌟]{0,20}"), 1..8
            ),
        ) {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let temp = tempfile::tempdir().unwrap();
                let ws = temp.path().join("ws");
                std::fs::create_dir_all(&ws).unwrap();
                let mut files: Vec<(String, String)> = Vec::new();
                for i in 0..n {
                    let (name, content) = &seeds[i % seeds.len()];
                    let rel = format!("thu-muc-{i}/{name}-{i}.txt");
                    files.push((rel, format!("{content}-{i}")));
                }
                for (rel, content) in &files {
                    let path = ws.join(rel);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, content).unwrap();
                }
                let store = store_in(temp.path(), 3);
                let checkpoint = store.capture(&ws).await.unwrap();
                // Phá hỏng toàn bộ: ghi đè nội dung khác.
                for (rel, content) in &files {
                    std::fs::write(ws.join(rel), format!("SAI: {content}")).unwrap();
                }
                checkpoint.restore(&ws).await.unwrap();
                let expected: std::collections::BTreeMap<String, String> = files
                    .iter()
                    .map(|(rel, content)| (rel.clone(), content.clone()))
                    .collect();
                proptest::prop_assert_eq!(read_tree(&ws), expected);
                Ok::<(), proptest::test_runner::TestCaseError>(())
            })
            .unwrap();
        }
    }
}
