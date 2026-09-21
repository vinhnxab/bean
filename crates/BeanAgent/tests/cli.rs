//! Test CLI end-to-end ở mức tiến trình (agents.md mục 21 M1: "`BeanAgent chat` chạy").
//!
//! Dùng `env!("CARGO_BIN_EXE_BeanAgent")` nên không cần thêm dev-dependency (như `assert_cmd`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::process::{Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_BeanAgent")
}

/// Chạy binary với stdin cho trước, trả về (mã thoát, stdout, stderr).
fn run_with_stdin(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("không chạy được binary BeanAgent");
    child
        .stdin
        .as_mut()
        .expect("stdin phải là pipe")
        .write_all(stdin.as_bytes())
        .expect("ghi stdin thất bại");
    let output = child.wait_with_output().expect("không lấy được output");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn help_lists_subcommands() {
    let output = Command::new(bin())
        .arg("--help")
        .output()
        .expect("chạy --help thất bại");
    assert!(output.status.success(), "--help phải thoát 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for needle in ["chat", "serve", "auth"] {
        assert!(
            stdout.contains(needle),
            "`--help` phải nhắc `{needle}`:\n{stdout}"
        );
    }
}

#[test]
fn version_flag_works() {
    let output = Command::new(bin())
        .arg("--version")
        .output()
        .expect("chạy --version thất bại");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("BeanAgent"));
}

#[test]
fn serve_and_auth_are_not_implemented_yet() {
    for args in [vec!["serve"], vec!["auth", "set-password"]] {
        let output = Command::new(bin())
            .args(&args)
            .output()
            .expect("chạy lệnh thất bại");
        assert!(!output.status.success(), "{args:?} phải thoát khác 0 ở M1");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("chưa được cài đặt"),
            "{args:?} stderr:\n{stderr}"
        );
    }
}

#[test]
fn chat_echoes_user_input_when_no_provider() {
    let (code, stdout, stderr) = run_with_stdin(&["chat"], "xin chào\n/exit\n");
    assert_eq!(code, 0, "chat phải thoát êm; stderr:\n{stderr}");
    assert!(stdout.contains("echo: xin chào"), "stdout:\n{stdout}");
}

#[test]
fn chat_runs_script_from_fake_llm_file() {
    let dir = tempfile::tempdir().expect("tạo thư mục tạm thất bại");
    let script = dir.path().join("kichban.json");
    std::fs::write(
        &script,
        r#"{"responses":[{"text":"Xin chào từ kịch bản"},{"text":"Tạm biệt"}]}"#,
    )
    .expect("ghi kịch bản thất bại");

    let (code, stdout, stderr) = run_with_stdin(
        &["chat", "--fake-llm", script.to_str().unwrap()],
        "chào\ncảm ơn\n/exit\n",
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(stdout.contains("Xin chào từ kịch bản"), "stdout:\n{stdout}");
    assert!(stdout.contains("Tạm biệt"), "stdout:\n{stdout}");
}

#[test]
fn chat_reports_missing_script_without_panic() {
    let (code, stdout, stderr) =
        run_with_stdin(&["chat", "--fake-llm", "/nonexistent/kichban.json"], "");
    assert_ne!(code, 0, "phải báo lỗi (không panic)");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("kịch bản"),
        "phải nêu rõ `--fake-llm`:\n{combined}"
    );
    assert!(
        !combined.contains("panicked"),
        "không được panic:\n{combined}"
    );
}

#[test]
fn chat_rejects_unknown_config_file() {
    let (code, _stdout, stderr) =
        run_with_stdin(&["--config", "/nonexistent/BeanAgent.toml", "chat"], "");
    assert_ne!(code, 0, "cấu hình không tồn tại phải là lỗi");
    assert!(stderr.contains("cấu hình"), "stderr:\n{stderr}");
}
