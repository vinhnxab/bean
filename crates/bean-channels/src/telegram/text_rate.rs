//! Cat tin an toan theo gioi han Telegram + gioi han tan suat + khu trung update.
//!
//! # Vì sao tách riêng
//!
//! Đây là **ba quy tắc bất biến của kênh Telegram** (mục 13), mỗi quy tắc chống một
//! cái hỏng khác nhau và cả ba đều là điều kiện sai mà không dễ phát hiện:
//!
//! * `split_text` — Telegram từ chối tin > 4096 ký tự; cắt giữa ký tự UTF-8 sẽ panic
//!   hoặc gửi ký tự hỏng.
//! * `ChatRateLimiter` — vượt hạn mức thì bot bị Telegram chặn, và việc này chỉ biết
//!   *sau khi* đã bị chặn.
//! * `UpdateDedup` — Telegram giao lại update khi mất ACK, chạy hai lần là thực thi
//!   tool hai lần.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

#[must_use]
pub fn split_text(text: &str, max_units: usize) -> Vec<String> {
    if text.is_empty() || max_units == 0 {
        return Vec::new();
    }
    if max_units == 1 {
        return text.chars().map(|ch| ch.to_string()).collect();
    }
    let mut parts = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut units = 0usize;
        let mut end = None;
        let mut boundary = None;
        for (relative, ch) in text[start..].char_indices() {
            let next = units.saturating_add(ch.len_utf16());
            if next > max_units {
                end = Some(start + relative);
                break;
            }
            units = next;
            let byte_end = start + relative + ch.len_utf8();
            if ch.is_whitespace() {
                boundary = Some(byte_end);
            }
            if units == max_units {
                end = Some(byte_end);
                break;
            }
        }
        let Some(end) = end else {
            parts.push(text[start..].to_owned());
            break;
        };
        let cut = boundary
            .filter(|value| *value > start && *value < end)
            .unwrap_or(end);
        parts.push(text[start..cut].to_owned());
        start = cut;
    }
    parts
}

/// Bounded, deterministic per-chat rate limiter.
#[derive(Debug)]
pub struct ChatRateLimiter {
    pub(super) limit: usize,
    pub(super) window: Duration,
    pub(super) entries: HashMap<i64, VecDeque<Instant>>,
}

impl ChatRateLimiter {
    /// Construct a limiter for a per-minute limit.
    #[must_use]
    pub fn new(limit: u32) -> Self {
        Self {
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
            window: Duration::from_secs(60),
            entries: HashMap::new(),
        }
    }

    /// Check and record one event at an explicit instant.
    pub fn allow(&mut self, chat_id: i64, now: Instant) -> bool {
        let cutoff = now.checked_sub(self.window);
        let entries = self.entries.entry(chat_id).or_default();
        if let Some(cutoff) = cutoff {
            while entries.front().is_some_and(|seen| *seen <= cutoff) {
                entries.pop_front();
            }
        }
        if entries.len() >= self.limit {
            return false;
        }
        entries.push_back(now);
        true
    }
}

/// Bounded update-id deduplicator.
#[derive(Debug)]
pub struct UpdateDedup {
    pub(super) seen: HashSet<u32>,
    pub(super) order: VecDeque<u32>,
    pub(super) capacity: usize,
}

impl UpdateDedup {
    /// Construct a deduplicator with a fixed memory bound.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            seen: HashSet::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    /// Return `true` only for the first occurrence of an update id.
    pub fn accept(&mut self, update_id: u32) -> bool {
        if !self.seen.insert(update_id) {
            return false;
        }
        self.order.push_back(update_id);
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}
