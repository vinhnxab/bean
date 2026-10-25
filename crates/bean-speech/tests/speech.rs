//! Test cho `bean-speech`: fake, local (stub script), ElevenLabs (wiremock) và validate
//! cấu hình `[voice]` (M27). Không dùng mạng thật, không cần whisper/piper thật.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use bean_speech::{
    AudioInput, ElevenLabsSpeech, FakeSpeech, LocalSpeech, SpeechError, SpeechProvider,
};
use bean_types::config::{Config, SpeechProviderKind};
use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn audio_webm() -> AudioInput {
    AudioInput {
        data: b"fake-audio-bytes".to_vec(),
        mime: "audio/webm".to_string(),
    }
}

#[tokio::test]
async fn fake_speech_transcribes_and_synthesizes() {
    let fake = FakeSpeech::new("xin chào");
    assert_eq!(fake.transcribe(&audio_webm()).await.unwrap(), "xin chào");
    let out = fake.synthesize("chào bạn").await.unwrap();
    assert_eq!(out.mime, "audio/mpeg");
    assert_eq!(String::from_utf8_lossy(&out.data), "chào bạn");
}

/// Viết một file script executable (dùng làm whisper/piper stub) trong thư mục tạm.
fn write_script(dir: &tempfile::TempDir, name: &str, body: &str) -> String {
    use std::io::Write;
    let path = dir.path().join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(body.as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = file.metadata().unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm).unwrap();
    }
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn local_speech_runs_stub_binaries() {
    let dir = tempfile::tempdir().unwrap();
    let whisper = write_script(
        &dir,
        "whisper",
        r#"#!/bin/bash
base=""; prev=""
for a in "$@"; do [ "$prev" = "-of" ] && base="$a"; prev="$a"; done
echo "xin chào từ stub" > "${base}.txt"
"#,
    );
    let piper = write_script(
        &dir,
        "piper",
        r#"#!/bin/bash
out=""; prev=""
for a in "$@"; do [ "$prev" = "--output_file" ] && out="$a"; prev="$a"; done
cat > /dev/null
printf 'RIFF....WAVEfake' > "$out"
"#,
    );
    let speech = LocalSpeech::new(
        whisper,
        "model.bin".into(),
        piper,
        "voice.onnx".into(),
        "vi".into(),
        Duration::from_secs(10),
    );
    let text = speech.transcribe(&audio_webm()).await.unwrap();
    assert_eq!(text, "xin chào từ stub");
    let out = speech.synthesize("một hai ba").await.unwrap();
    assert_eq!(out.mime, "audio/wav");
    assert!(out.data.starts_with(b"RIFF"));
}

#[tokio::test]
async fn local_speech_reports_missing_binary() {
    let speech = LocalSpeech::new(
        "/nonexistent/whisper".into(),
        "m.bin".into(),
        "/nonexistent/piper".into(),
        "v.onnx".into(),
        "vi".into(),
        Duration::from_secs(5),
    );
    let err = speech.transcribe(&audio_webm()).await.unwrap_err();
    assert!(matches!(err, SpeechError::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn elevenlabs_transcribes_and_synthesizes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/speech-to-text"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "text": "hỏi thời tiết hà nội"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/text-to-speech/vid123"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "audio/mpeg")
                .set_body_bytes(vec![1u8, 2, 3, 4]),
        )
        .mount(&server)
        .await;

    let speech = ElevenLabsSpeech::new(
        server.uri(),
        SecretString::from("key".to_string()),
        "scribe_v2".into(),
        "eleven_multilingual_v2".into(),
        "vid123".into(),
        "vi".into(),
        Duration::from_secs(10),
    )
    .unwrap();

    let text = speech.transcribe(&audio_webm()).await.unwrap();
    assert_eq!(text, "hỏi thời tiết hà nội");
    let out = speech.synthesize("trời hôm nay đẹp").await.unwrap();
    assert_eq!(out.mime, "audio/mpeg");
    assert_eq!(out.data, vec![1, 2, 3, 4]);
}

#[test]
fn validate_voice_rejects_shared_credential() {
    let mut config = Config::default();
    config.voice.enabled = true;
    config.voice.provider = SpeechProviderKind::Api;
    config.voice.api_key_env = "ANTHROPIC_API_KEY".into();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("trùng"), "{err}");
}

#[test]
fn validate_voice_local_requires_models() {
    let mut config = Config::default();
    config.voice.enabled = true;
    config.voice.provider = SpeechProviderKind::Local;
    config.voice.whisper_model = "".into();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("whisper_model"), "{err}");
}

#[test]
fn validate_voice_rejects_oversized_speak_chars() {
    let mut config = Config::default();
    config.voice.enabled = true;
    config.voice.max_speak_chars = 5000;
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("max_speak_chars"), "{err}");
}

#[test]
fn build_speech_returns_none_when_disabled() {
    let config = Config::default();
    assert!(bean_speech::build_speech(&config, None).unwrap().is_none());
}

#[test]
fn build_speech_api_requires_key() {
    let mut config = Config::default();
    config.voice.enabled = true;
    config.voice.provider = SpeechProviderKind::Api;
    let err = bean_speech::build_speech(&config, None)
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(err, SpeechError::NotConfigured(_)), "{err:?}");
}

// Toggle từ UI (PATCH /api/voice) phải bật lại được không cần restart: provider được
// dựng khi `[voice]` khai báo đủ, bất kể cờ `enabled` (serve chỉ coi `enabled` là
// trạng thái runtime, kiểm tra tại từng request trong bean-web).
#[test]
fn build_speech_builds_when_disabled_but_configured() {
    let mut config = Config::default();
    config.voice.enabled = false;
    config.voice.whisper_model = "/tmp/ggml-vi.bin".into();
    config.voice.piper_model = "/tmp/vi.onnx".into();
    assert!(config.voice_configured());
    let provider = bean_speech::build_speech(&config, None).unwrap();
    assert!(provider.is_some(), "provider phải dựng được dù seed tắt");
}

#[test]
fn build_speech_none_when_fields_incomplete() {
    let mut config = Config::default();
    config.voice.enabled = true;
    // Thiếu model local ⇒ chưa cấu hình đủ, dù cờ enabled bật.
    let provider = bean_speech::build_speech(&config, None).unwrap();
    assert!(provider.is_none());
}

#[test]
fn truncate_utf8_boundary() {
    assert_eq!(bean_speech::truncate_utf8("abc", 10), "abc");
    assert_eq!(bean_speech::truncate_utf8("Xin chào", 3), "Xin");
}
