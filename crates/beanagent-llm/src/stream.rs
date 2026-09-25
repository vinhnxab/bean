//! Khung stream SSE tối giản dùng chung cho các provider (M17).
//!
//! Parser cố tình không phụ thuộc SDK: reqwest chỉ cấp các byte chunk tùy ý, SSE quy
//! định event khi gặp dòng trống. Việc decode cả `data` nhiều dòng và CRLF giúp wiremock
//! cũng như gateway thực dùng chung một đường parse.

use std::collections::VecDeque;

use futures_util::{Stream, StreamExt, stream};

use beanagent_types::LlmDelta;

use crate::LlmError;

/// Một event SSE đã hoàn chỉnh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// Tên event nếu provider gửi trường `event:`.
    pub event: Option<String>,
    /// Các dòng `data:` đã nối bằng newline.
    pub data: String,
}

/// Giới hạn buffer/event để endpoint hỏng không làm tăng bộ nhớ vô hạn.
const MAX_SSE_EVENT_BYTES: usize = 8 * 1024 * 1024;

/// Decoder SSE tăng dần, chịu được chunk cắt giữa CRLF hoặc giữa UTF-8.
#[derive(Debug, Default)]
pub(crate) struct SseDecoder {
    buffer: Vec<u8>,
    event: Option<String>,
    data_lines: Vec<String>,
    has_data: bool,
}

impl SseDecoder {
    /// Nạp một chunk byte và trả về mọi event đã đủ dữ liệu.
    ///
    /// # Errors
    /// Trả lỗi khi event vượt giới hạn hoặc event SSE không phải UTF-8 hợp lệ.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, LlmError> {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > MAX_SSE_EVENT_BYTES {
            return Err(LlmError::Decode(
                "event SSE vượt giới hạn 8 MiB".to_string(),
            ));
        }

        let mut events = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if let Some(event) = self.process_line(&line)? {
                events.push(event);
            }
        }
        Ok(events)
    }

    /// Flush event cuối nếu HTTP stream kết thúc ngay sau dòng data.
    ///
    /// # Errors
    /// Như [`SseDecoder::push`].
    pub(crate) fn finish(&mut self) -> Result<Vec<SseEvent>, LlmError> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let mut line = std::mem::take(&mut self.buffer);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if let Some(event) = self.process_line(&line)? {
                events.push(event);
            }
        }
        if let Some(event) = self.dispatch() {
            events.push(event);
        }
        Ok(events)
    }

    fn process_line(&mut self, line: &[u8]) -> Result<Option<SseEvent>, LlmError> {
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if line.first() == Some(&b':') {
            return Ok(None);
        }
        let line = std::str::from_utf8(line).map_err(|error| {
            LlmError::Decode(format!("event SSE không phải UTF-8 hợp lệ: {error}"))
        })?;
        let Some((field, raw_value)) = line.split_once(':') else {
            // Field không có colon có value rỗng theo SSE; các field này không dùng.
            return Ok(None);
        };
        let value = raw_value.strip_prefix(' ').unwrap_or(raw_value);
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                if self.has_data {
                    self.data_lines.push("\n".to_string());
                }
                self.data_lines.push(value.to_string());
                self.has_data = true;
            }
            _ => {}
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        if !self.has_data {
            self.event = None;
            return None;
        }
        let event = SseEvent {
            event: self.event.take(),
            data: self.data_lines.join(""),
        };
        self.data_lines.clear();
        self.has_data = false;
        Some(event)
    }
}

/// Parser trạng thái riêng của từng provider.
pub(crate) trait SseEventParser: Send + 'static {
    /// Chuyển một event SSE thành zero hoặc nhiều delta.
    fn parse_event(&mut self, event: SseEvent) -> Result<Vec<LlmDelta>, LlmError>;

    /// Stream đã có terminator hợp lệ chưa?
    fn is_done(&self) -> bool;

    /// Kiểm tra trạng thái bắt buộc khi HTTP body kết thúc.
    fn finish(&mut self) -> Result<(), LlmError>;
}

struct DecodeState<P> {
    response: reqwest::Response,
    decoder: SseDecoder,
    parser: P,
    pending: VecDeque<LlmDelta>,
    done: bool,
}

/// Đọc response SSE và trả stream delta có thể lỗi giữa chừng.
pub(crate) fn decode_sse<P>(
    response: reqwest::Response,
    parser: P,
) -> impl Stream<Item = Result<LlmDelta, LlmError>>
where
    P: SseEventParser,
{
    stream::unfold(
        DecodeState {
            response,
            decoder: SseDecoder::default(),
            parser,
            pending: VecDeque::new(),
            done: false,
        },
        |mut state| async move {
            loop {
                if let Some(delta) = state.pending.pop_front() {
                    return Some((Ok(delta), state));
                }
                if state.done {
                    return None;
                }

                match state.response.chunk().await {
                    Ok(Some(bytes)) => {
                        let events = match state.decoder.push(&bytes) {
                            Ok(events) => events,
                            Err(error) => {
                                state.done = true;
                                return Some((Err(error), state));
                            }
                        };
                        for event in events {
                            match state.parser.parse_event(event) {
                                Ok(deltas) => state.pending.extend(deltas),
                                Err(error) => {
                                    state.pending.clear();
                                    state.done = true;
                                    return Some((Err(error), state));
                                }
                            }
                        }
                        if state.parser.is_done() {
                            state.done = true;
                        }
                    }
                    Err(error) => {
                        state.done = true;
                        return Some((Err(crate::http::to_transport_error(error)), state));
                    }
                    Ok(None) => {
                        let events = match state.decoder.finish() {
                            Ok(events) => events,
                            Err(error) => {
                                state.done = true;
                                return Some((Err(error), state));
                            }
                        };
                        for event in events {
                            match state.parser.parse_event(event) {
                                Ok(deltas) => state.pending.extend(deltas),
                                Err(error) => {
                                    state.done = true;
                                    return Some((Err(error), state));
                                }
                            }
                        }
                        if let Err(error) = state.parser.finish() {
                            state.done = true;
                            return Some((Err(error), state));
                        }
                        state.done = true;
                    }
                }
            }
        },
    )
    .boxed()
}
