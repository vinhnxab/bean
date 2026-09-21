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
    run_with_stdin_env(args, stdin, &[])
}

/// Như `run_with_stdin`, kèm việc **xoá** biến môi trường (phòng khi máy test có sẵn
/// `ANTHROPIC_API_KEY` — test phải độc lập với môi trường đó).
fn run_with_stdin_env(args: &[&str], stdin: &str, env_remove: &[&str]) -> (i32, String, String) {
    let mut command = Command::new(bin());
    command.args(args);
    for variable in env_remove {
        command.env_remove(variable);
    }
    let mut child = command
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
fn chat_without_provider_reports_missing_env_var_clearly() {
    // M2: `chat` không còn echo mode — không có `--fake-llm` và không có API key
    // (mặc định provider `anthropic`, `api_key_env = ANTHROPIC_API_KEY`) phải thoát lỗi
    // với thông báo nêu rõ TÊN biến môi trường (agents.md mục 15.6), không panic.
    let (_code, stdout, stderr) =
        run_with_stdin_env(&["chat"], "xin chào\n/exit\n", &["ANTHROPIC_API_KEY"]);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("ANTHROPIC_API_KEY"),
        "phải nêu tên biến môi trường:\n{combined}"
    );
    assert!(
        !combined.contains("panicked"),
        "không được panic:\n{combined}"
    );
}

/// E2E M2: `chat` với provider **thật** (`openai_compat`) trỏ vào mock server local
/// (wiremock chạy trong tiến trình test) — phủ trọn đường config → factory → HTTP → parse,
/// không cần mạng ngoài (agents.md mục 21 M2: "chat với provider thật, 1 lượt, không tool").
#[tokio::test]
async fn chat_with_openai_compat_provider_round_trip() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, ResponseTemplate};

    let server = wiremock::MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("Authorization", "Bearer test-key-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "Chào bạn, tôi là BeanAgent." }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 9, "completion_tokens": 7 }
        })))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("BeanAgent.toml");
    std::fs::write(
        &config_path,
        format!(
            "[llm]\nprovider = \"openai_compat\"\nmodel = \"test-model\"\napi_key_env = \"BEANAGENT_TEST_KEY\"\nbase_url = \"{}/v1\"\nmax_tokens = 64\n",
            server.uri()
        ),
    )
    .unwrap();

    let config_arg = format!("--config={}", config_path.display());
    let mut child = Command::new(bin())
        .args(["chat", &config_arg])
        .env("BEANAGENT_TEST_KEY", "test-key-123")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all("hỏi thử\n/exit\n".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(output.status.success(), "stderr:\n{stderr}");
    assert!(
        stdout.contains("Chào bạn, tôi là BeanAgent."),
        "phải in phản hồi của provider:\n{stdout}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// Provider thật trả 500 liên tục → CLI in lỗi provider ra stderr nhưng **không** thoát lỗi
/// (phiên chat vẫn tiếp tục cho tới `/exit`).
#[tokio::test]
async fn chat_survives_provider_error() {
    use wiremock::matchers::method;
    use wiremock::{Mock, ResponseTemplate};

    let server = wiremock::MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("BeanAgent.toml");
    std::fs::write(
        &config_path,
        format!(
            "[llm]\nprovider = \"openai_compat\"\nmodel = \"m\"\napi_key_env = \"BEANAGENT_TEST_KEY\"\nbase_url = \"{}/v1\"\nmax_tokens = 64\n",
            server.uri()
        ),
    )
    .unwrap();

    let config_arg = format!("--config={}", config_path.display());
    let mut child = Command::new(bin())
        .args(["chat", &config_arg])
        .env("BEANAGENT_TEST_KEY", "k")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all("xin chào\n/exit\n".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "lỗi provider không được làm hỏng phiên:\n{stderr}"
    );
    assert!(stderr.contains("lỗi provider"), "stderr:\n{stderr}");
    // 500 được retry đủ 3 lần → tổng 4 request (chính sách mục 5).
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
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
