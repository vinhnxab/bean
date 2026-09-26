//! Test nhóm tool file qua `CapWorkspace` thật (agents.md mục 7.3/20) —
//! suite tương ứng của `FsWorkspace` (M3) đã bị xoá cùng cài đặt tạm thời.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use beanagent_security::CapWorkspace;
use beanagent_security::untrusted::{CLOSE_TAG, OPEN_TAG};
use beanagent_tools::ToolCtx;
use beanagent_tools::ToolRegistry;
use beanagent_tools::builtin::file_tools;
use beanagent_types::Risk;
use tokio_util::sync::CancellationToken;

fn files_registry(dir: &tempfile::TempDir) -> ToolRegistry {
    let ws = Arc::new(CapWorkspace::open(dir.path().to_path_buf()).unwrap());
    let mut reg = ToolRegistry::with_workspace(ws);
    for t in file_tools() {
        reg.register(t).unwrap();
    }
    reg
}

fn ctx_of(reg: &ToolRegistry) -> ToolCtx {
    ToolCtx::for_project(
        reg.workspace().unwrap(),
        beanagent_types::SessionId::new(1),
        CancellationToken::new(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
}

async fn call(
    reg: &ToolRegistry,
    name: &str,
    args: serde_json::Value,
) -> Result<String, beanagent_tools::ToolError> {
    let tool = reg.get(name).unwrap();
    tool.call(&ctx_of(reg), args).await
}

#[tokio::test]
async fn read_write_and_grep_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let reg = files_registry(&dir);
    call(
        &reg,
        "write_file",
        serde_json::json!({"path": "hello.txt", "content": "Xin chào 🦀\nDòng thứ hai ツ\n"}),
    )
    .await
    .unwrap();

    let got = call(
        &reg,
        "read_file",
        serde_json::json!({"path": "hello.txt", "limit": 100}),
    )
    .await
    .unwrap();
    assert!(got.contains("Xin chào"));
    assert!(got.contains("🦀"));

    let hits = call(
        &reg,
        "grep",
        serde_json::json!({"pattern": "chào", "limit": 10}),
    )
    .await
    .unwrap();
    assert!(hits.contains("hello.txt"), "{hits}");
}

#[tokio::test]
async fn read_offset_seeks_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let reg = files_registry(&dir);
    call(
        &reg,
        "write_file",
        serde_json::json!({"path": "t.txt", "content": "ABCDEabcde"}),
    )
    .await
    .unwrap();
    let part = call(
        &reg,
        "read_file",
        serde_json::json!({"path": "t.txt", "offset": 3, "limit": 5}),
    )
    .await
    .unwrap();
    // Mục 15.4: output của `read_file` được bọc `<untrusted_content>`; phần **bên
    // trong** thẻ phải đúng 5 ký tự tại offset 3 (không nới lỏng kiểm tra offset).
    let inner = part
        .strip_prefix(OPEN_TAG)
        .and_then(|rest| rest.strip_suffix(CLOSE_TAG))
        .unwrap_or_default();
    assert_eq!(inner.trim_matches('\n'), "DEabc", "output: {part}");
}

#[tokio::test]
async fn read_missing_file_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let reg = files_registry(&dir);
    let err = call(
        &reg,
        "read_file",
        serde_json::json!({"path": "nope.txt", "limit": 100}),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, beanagent_tools::ToolError::NotFound(_)),
        "{err:?}"
    );
}

/// Rủi ro của các tool file đúng theo mục 7.3.
#[tokio::test]
async fn file_tool_risks_match_spec() {
    let dir = tempfile::tempdir().unwrap();
    let reg = files_registry(&dir);
    let empty = serde_json::json!({});
    assert_eq!(reg.get("read_file").unwrap().risk(&empty), Risk::Safe);
    assert_eq!(reg.get("list_dir").unwrap().risk(&empty), Risk::Safe);
    assert_eq!(reg.get("glob").unwrap().risk(&empty), Risk::Safe);
    assert_eq!(reg.get("grep").unwrap().risk(&empty), Risk::Safe);
    assert_eq!(reg.get("write_file").unwrap().risk(&empty), Risk::Confirm);
    assert_eq!(reg.get("edit_file").unwrap().risk(&empty), Risk::Confirm);
}
