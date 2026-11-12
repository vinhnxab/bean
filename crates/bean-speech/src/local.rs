//! Chế độ local: chạy `whisper-cli` (STT) + `piper` (TTS) như tiến trình con (M27 mục 9.1).
//!
//! # Vì sao là tiến trình con chứ không binding Rust
//!
//! Whisper.cpp và Piper đều dựa ONNX/C++; build chúng từ Rust buộc workspace kéo toolchain
//! C++ và hàng trăm MB dependency. Gọi binary người vận hành đã cài sẵn giữ binary Bean
//! nhẹ, khớp tinh thần "một binary" (agents.md mục 1). Đổi lại: thiếu binary/model thì phải
//! báo lỗi rõ — không âm thầm fallback (D6.4).
//!
//! # Lưu ý định dạng audio
//!
//! `whisper-cli` đọc file WAV tốt nhất; trình duyệt thường ghi `audio/webm`. Nếu bản build
//! whisper của bạn không giải mã webm, hãy để trình duyệt ghi `audio/wav` hoặc đặt ffmpeg
//! vào đường trung gian — đây là trách nhiệm cấu hình của người vận hành.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::{AudioInput, AudioOutput, SpeechError, SpeechProvider};

/// Spawn tiến trình con, thử lại khi `ETXTBSY` (os error 26, "Text file busy").
///
/// Giống `bean-security::spawn_with_busy_retry` (known-issues "CI flake Text
/// file busy"): exec một file vừa được ghi khi fd ghi còn mở ở đâu đó bị kernel
/// từ chối **tạm thời** — tái hiện ~1/20 lần khi test song song. Lỗi khác (ENOENT…)
/// là vĩnh viễn, trả ngay. Retry ≤ 3 lần, backoff 10/20 ms — nhỏ so với timeout.
async fn spawn_with_busy_retry(
    command: &mut Command,
    label: &str,
) -> Result<tokio::process::Child, SpeechError> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut backoff = Duration::from_millis(10);
    let mut attempt = 1;
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && attempt < MAX_ATTEMPTS =>
            {
                attempt += 1;
                tokio::time::sleep(backoff).await;
                backoff *= 2;
            }
            Err(error) => {
                return Err(SpeechError::NotFound(format!(
                    "không chạy được `{label}`: {error}"
                )));
            }
        }
    }
}

/// STT/TTS chạy cục bộ bằng tiến trình con.
pub struct LocalSpeech {
    whisper_command: String,
    whisper_model: String,
    piper_command: String,
    piper_model: String,
    language: String,
    timeout: Duration,
}

impl LocalSpeech {
    /// Dựng provider local từ cấu hình.
    #[must_use]
    pub fn new(
        whisper_command: String,
        whisper_model: String,
        piper_command: String,
        piper_model: String,
        language: String,
        timeout: Duration,
    ) -> Self {
        Self {
            whisper_command,
            whisper_model,
            piper_command,
            piper_model,
            language,
            timeout,
        }
    }

    /// Phần mở rộng file tạm theo MIME của audio đầu vào.
    fn extension_for(mime: &str) -> &'static str {
        let mime = mime.to_ascii_lowercase();
        if mime.contains("wav") {
            "wav"
        } else if mime.contains("mpeg") || mime.contains("mp3") {
            "mp3"
        } else if mime.contains("ogg") {
            "ogg"
        } else if mime.contains("mp4") || mime.contains("m4a") {
            "m4a"
        } else {
            "webm"
        }
    }
}

#[async_trait::async_trait]
impl SpeechProvider for LocalSpeech {
    async fn transcribe(&self, audio: &AudioInput) -> Result<String, SpeechError> {
        let whisper_command = self.whisper_command.clone();
        let whisper_model = self.whisper_model.clone();
        let language = self.language.clone();
        let ext = Self::extension_for(&audio.mime).to_string();
        let data = audio.data.clone();
        let timeout = self.timeout;

        let run = async move {
            let dir = tempfile::tempdir().map_err(|e| SpeechError::NotFound(e.to_string()))?;
            let base = dir.path().join("audio");
            let input = base.with_extension(&ext);
            tokio::fs::write(&input, &data)
                .await
                .map_err(|e| SpeechError::Failed(format!("ghi file audio tạm: {e}")))?;
            // whisper.cpp: `-otxt -of <base>` ghi transcript ra `<base>.txt`; `-nt` bỏ timestamp.
            let mut command = Command::new(&whisper_command);
            command
                .arg("-m")
                .arg(&whisper_model)
                .arg("-f")
                .arg(&input)
                .arg("-l")
                .arg(&language)
                .arg("-nt")
                .arg("-otxt")
                .arg("-of")
                .arg(&base)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let child = spawn_with_busy_retry(&mut command, &whisper_command).await?;
            let output = child
                .wait_with_output()
                .await
                .map_err(|e| SpeechError::Failed(format!("chờ `{whisper_command}`: {e}")))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(SpeechError::Failed(format!(
                    "`{whisper_command}` thoát mã {}: {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                )));
            }
            let txt = base.with_extension("txt");
            let text = tokio::fs::read_to_string(&txt).await.map_err(|e| {
                SpeechError::Failed(format!("không đọc được transcript {}: {e}", txt.display()))
            })?;
            Ok(text.trim().to_string())
        };

        tokio::time::timeout(timeout, run)
            .await
            .map_err(|_| SpeechError::Timeout(timeout))?
    }

    async fn synthesize(&self, text: &str) -> Result<AudioOutput, SpeechError> {
        let piper_command = self.piper_command.clone();
        let piper_model = self.piper_model.clone();
        let text = text.to_string();
        let timeout = self.timeout;

        let run = async move {
            let dir = tempfile::tempdir().map_err(|e| SpeechError::NotFound(e.to_string()))?;
            let out = dir.path().join("out.wav");
            let mut command = Command::new(&piper_command);
            command
                .arg("-m")
                .arg(&piper_model)
                .arg("--output_file")
                .arg(&out)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            let mut child = spawn_with_busy_retry(&mut command, &piper_command).await?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(text.as_bytes())
                    .await
                    .map_err(|e| SpeechError::Failed(format!("ghi stdin piper: {e}")))?;
                drop(stdin);
            }
            let output = child
                .wait_with_output()
                .await
                .map_err(|e| SpeechError::Failed(format!("chờ piper: {e}")))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(SpeechError::Failed(format!(
                    "`{piper_command}` thoát mã {}: {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                )));
            }
            let data = tokio::fs::read(&out).await.map_err(|e| {
                SpeechError::Failed(format!("không đọc được audio {}: {e}", out.display()))
            })?;
            Ok(AudioOutput {
                data,
                mime: "audio/wav".to_string(),
            })
        };

        tokio::time::timeout(timeout, run)
            .await
            .map_err(|_| SpeechError::Timeout(timeout))?
    }
}
