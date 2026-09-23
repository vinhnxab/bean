//! Test sandbox docker thật (agents.md mục 15.2/20): container chỉ thấy workspace,
//! không có mạng khi `network=false`, timeout giết được container.
//!
//! Các test này **tự skip** (in lý do, return) khi docker daemon/image không sẵn sàng —
//! `make check` không đỏ trên máy không có docker; chạy thật khi có docker.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use beanagent_security::{Sandbox, SandboxError};
use beanagent_types::config::{SandboxConfig, SandboxMode};
use tokio_util::sync::CancellationToken;

/// Image test có sẵn trong môi trường dev (không pull — tránh phụ thuộc mạng).
const TEST_IMAGE: &str = "python:3.12-slim";

/// Kiểm tra daemon docker + image sẵn sàng (timeout 10s mỗi bước).
async fn docker_ready() -> bool {
    let info = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::process::Command::new("docker")
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !info {
        return false;
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::process::Command::new("docker")
            .args(["image", "inspect", TEST_IMAGE])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

fn docker_sandbox(workspace: std::path::PathBuf, network: bool) -> Sandbox {
    Sandbox::new(
        SandboxConfig {
            mode: SandboxMode::Docker,
            image: TEST_IMAGE.to_string(),
            network,
            timeout_seconds: 5,
            ..SandboxConfig::default()
        },
        workspace,
    )
}

/// Đếm container `beanagent-sbx-*` đang chạy.
async fn running_sbx_containers() -> usize {
    let Ok(out) = tokio::process::Command::new("docker")
        .args([
            "ps",
            "--filter",
            "name=beanagent-sbx-",
            "--format",
            "{{.Names}}",
        ])
        .output()
        .await
    else {
        return usize::MAX;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

/// 1. Container chỉ thấy workspace — file ở thư mục cha (ngoài workspace) là vô hình.
#[tokio::test]
async fn docker_sandbox_sees_only_workspace() {
    if !docker_ready().await {
        eprintln!("SKIP docker_sandbox_sees_only_workspace: docker/image không sẵn sàng");
        return;
    }
    let parent = tempfile::tempdir().unwrap();
    let ws_dir = parent.path().join("ws");
    std::fs::create_dir(&ws_dir).unwrap();
    std::fs::write(ws_dir.join("in.txt"), "trong workspace").unwrap();
    // File NGOÀI workspace (cùng thư mục cha) — container không được thấy.
    let outside = parent.path().join("outside-secret.txt");
    std::fs::write(&outside, "ngoai").unwrap();

    let sandbox = docker_sandbox(ws_dir, false);
    let cancel = CancellationToken::new();

    // Thấy file trong workspace, không thấy file ngoài qua đường dẫn tương đối.
    let out = sandbox
        .run(
            "cat /workspace/in.txt && test ! -e /workspace/../outside-secret.txt",
            Duration::from_secs(15),
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(0), "stderr: {}", out.stderr);
    assert!(out.stdout.contains("trong workspace"));
    // Absolute path của host cũng vô hình trong container.
    let host_path = outside.to_string_lossy().to_string();
    let probe = sandbox
        .run(
            &format!("test ! -e '{host_path}'"),
            Duration::from_secs(15),
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(
        probe.exit_code,
        Some(0),
        "container thấy file host: {host_path}"
    );
}

/// 2. Không có mạng khi `network=false` (mặc định): `/proc/net/dev` không có eth0,
/// DNS không phân giải được.
#[tokio::test]
async fn docker_sandbox_has_no_network_when_disabled() {
    if !docker_ready().await {
        eprintln!("SKIP docker_sandbox_has_no_network_when_disabled: docker/image không sẵn sàng");
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let sandbox = docker_sandbox(ws.path().to_path_buf(), false);
    let cancel = CancellationToken::new();

    let out = sandbox
        .run(
            "cat /proc/net/dev; echo '---'; getent hosts example.com || echo NO_DNS",
            Duration::from_secs(20),
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(out.exit_code, Some(0), "stderr: {}", out.stderr);
    let combined = format!("{}{}", out.stdout, out.stderr);
    let (ifaces, tail) = combined.split_once("---").unwrap_or((&combined, ""));
    // Chỉ loopback — không eth0/tên interface mạng container.
    assert!(
        !ifaces.contains("eth0"),
        "container có interface mạng: {ifaces}"
    );
    // DNS không phân giải được với --network none.
    assert!(tail.contains("NO_DNS"), "{combined}");
}

/// 3. Timeout giết được container: sleep 30 với timeout 3s → lỗi Timeout nhanh,
/// không còn container `beanagent-sbx-*` sống sót.
#[tokio::test]
async fn docker_timeout_kills_container() {
    if !docker_ready().await {
        eprintln!("SKIP docker_timeout_kills_container: docker/image không sẵn sàng");
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let sandbox = docker_sandbox(ws.path().to_path_buf(), false);
    let cancel = CancellationToken::new();

    let before = running_sbx_containers().await;
    let started = std::time::Instant::now();
    let err = sandbox
        .run("sleep 30", Duration::from_secs(3), &cancel)
        .await
        .unwrap_err();
    assert!(matches!(err, SandboxError::Timeout(_)), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "không kill đủ nhanh: {:?}",
        started.elapsed()
    );

    // Chờ docker kill + --rm remove xong rồi kiểm tra.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = running_sbx_containers().await;
    assert!(
        after <= before,
        "container mồ côi còn sống: trước={before}, sau={after}"
    );
}
