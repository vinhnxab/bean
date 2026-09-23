//! Test path jail của `CapWorkspace` (agents.md mục 15.1/20):
//! đường dẫn tuyệt đối, `..`, symlink escape, roundtrip I/O.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use beanagent_security::CapWorkspace;
use beanagent_tools::{ToolError, WorkspaceFs};

fn ws_in(dir: &tempfile::TempDir) -> CapWorkspace {
    CapWorkspace::open(dir.path().to_path_buf()).unwrap()
}

/// File "canary" nằm NGOÀI workspace — không bao giờ được đọc/ghi qua path jail.
struct Canary {
    _tmp: tempfile::TempDir,
    path: std::path::PathBuf,
}

impl Canary {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("secret-outside.txt");
        std::fs::write(&path, "SECRET-NGOAI-WORKSPACE").unwrap();
        Self { _tmp: tmp, path }
    }
}

#[test]
fn rejects_absolute_paths() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_in(&dir);
    for p in ["/etc/passwd", "/etc/shadow", "/"] {
        assert!(
            matches!(ws.resolve(p), Err(ToolError::NotInWorkspace(_))),
            "{p} phải bị chặn"
        );
        assert!(ws.read_text(p).is_err(), "{p} phải bị chặn khi đọc");
        assert!(ws.write_text(p, "x").is_err(), "{p} phải bị chặn khi ghi");
    }
}

#[test]
fn rejects_parent_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_in(&dir);
    for p in ["..", "../x", "a/../../b", "a/b/../../../c"] {
        assert!(
            matches!(ws.resolve(p), Err(ToolError::NotInWorkspace(_))),
            "{p} phải bị chặn"
        );
    }
    assert!(ws.resolve("./ok.txt").is_ok());
}

#[test]
fn rejects_nul_byte() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_in(&dir);
    assert!(ws.resolve("a\0b").is_err());
}

#[cfg(unix)]
#[test]
fn symlink_file_escape_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let canary = Canary::new();
    let ws = ws_in(&dir);

    // Symlink trong workspace trỏ ra file ngoài.
    std::os::unix::fs::symlink(&canary.path, dir.path().join("leak.txt")).unwrap();

    // cap-std: symlink trỏ RA NGOÀI Dir → lỗi, không bao giờ đọc được nội dung canary.
    let blocked = matches!(
        ws.read_text("leak.txt"),
        Err(ToolError::Io(_) | ToolError::NotInWorkspace(_) | ToolError::NotFound(_))
    );
    assert!(blocked, "symlink trỏ ngoài workspace phải bị chặn khi đọc");
    // Ghi qua symlink cũng phải lỗi (không phá được file ngoài).
    assert!(ws.write_text("leak.txt", "defaced").is_err());
    // walk bỏ symlink — glob chỉ liệt kê file thường.
    ws.write_text("real.txt", "bình thường").unwrap();
    let files = ws.glob("**/*", 100).unwrap();
    assert_eq!(files, vec!["real.txt".to_string()]);
    // File canary không đổi.
    assert_eq!(
        std::fs::read_to_string(&canary.path).unwrap(),
        "SECRET-NGOAI-WORKSPACE"
    );
}

#[cfg(unix)]
#[test]
fn symlink_dir_escape_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("hidden.txt"), "ngoài").unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
    let ws = ws_in(&dir);

    // Đọc file qua symlink-dir phải lỗi.
    assert!(ws.read_text("out/hidden.txt").is_err());
    // list_dir không đi xuyên symlink-dir ra ngoài.
    assert!(ws.list_dir("out").is_err());
    // glob không liệt kê file ngoài.
    let files = ws.glob("**/*.txt", 100).unwrap();
    assert!(
        files.iter().all(|f| !f.contains("hidden.txt")),
        "không được liệt kê file ngoài: {files:?}"
    );
}

#[test]
fn read_write_edit_roundtrip_and_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_in(&dir);
    ws.write_text("sub/a.txt", "hello\n").unwrap();
    assert_eq!(ws.read_text("sub/a.txt").unwrap(), "hello\n");
    ws.edit_unique("sub/a.txt", "hello", "xin chào 🦀").unwrap();
    // Giữ nguyên phần còn lại của file (ngoài chuỗi được thay).
    assert_eq!(ws.read_text("sub/a.txt").unwrap(), "xin chào 🦀\n");
    // edit không duy nhất / không tồn tại → lỗi.
    assert!(ws.edit_unique("sub/a.txt", "không-có", "x").is_err());
    ws.write_text("f.txt", "ab ab").unwrap();
    assert!(ws.edit_unique("f.txt", "ab", "x").is_err());
    assert!(ws.edit_unique("f.txt", "ab ab", "x").is_ok());
}

#[test]
fn read_range_respects_offset_limit() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_in(&dir);
    ws.write_text("t.txt", "ABCDEabcde").unwrap();
    assert_eq!(ws.read_range("t.txt", 3, 5).unwrap(), b"DEabc");
    assert_eq!(ws.read_range("t.txt", 100, 5).unwrap(), b"");
    assert!(matches!(
        ws.read_range("thiếu.txt", 0, 5),
        Err(ToolError::NotFound(_))
    ));
}

#[test]
fn glob_and_grep_walk_tree() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_in(&dir);
    ws.write_text("src/main.rs", "fn main() {}\n").unwrap();
    ws.write_text("src/lib/deep.rs", "const HÈ: &str = \"việt\";\n")
        .unwrap();
    ws.write_text("notes.md", "tìm em 🦀\n").unwrap();

    let rs = ws.glob("**/*.rs", 50).unwrap();
    assert_eq!(rs, vec!["src/lib/deep.rs", "src/main.rs"]);
    assert!(ws.glob("*.rs", 50).unwrap().is_empty(), "* không vượt `/`");

    let hits = ws.grep("việt", None, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "src/lib/deep.rs");
    let scoped = ws.grep("main", Some("src"), 10).unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].path, "src/main.rs");
}
