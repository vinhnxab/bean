//! Bộ giới hạn tần suất dùng chung — **tách ra từ `POST /api/auth/login`** (K24).
//!
//! # Vì sao ở đây, không viết thuật toán thứ hai
//!
//! `POST /api/auth/login` đã có sẵn cơ chế khoá tăng dần theo IP: cửa sổ 60 giây, khoá khi
//! đạt ngưỡng thất bại, thời gian khoá `1 << min(n - ngưỡng, 8)` giây, trần 300 giây.
//! K24 đòi giới hạn tần suất cho `POST /mcp` mà **không** phát minh thuật toán mới — nên
//! toàn bộ phần toán học ở trên nằm ở đúng một chỗ: [`RateLimiter`]. `AuthService` (web)
//! và `McpRateLimiter` (MCP) chỉ khác ở **khoá** (IP hay token) và ở **lúc nào** bộ đếm
//! được tăng — đó là chi tiết của người gọi, không phải của thuật toán.
//!
//! # Vì sao khoá theo `String` thay vì `IpAddr`
//!
//! Login chỉ cần khoá theo IP; MCP cần khoá theo **cả** IP lẫn dấu vân tay token
//! (`hash_token`, không bao giờ là token thô). Một kiểu khoá duy nhất giữ cho hai nơi dùng
//! chung đúng một cấu trúc dữ liệu, và tiền tố khoá (`ip:`/`token:`) hiện ngay trong log.
//!
//! # Chặn tự cạn bộ nhớ
//!
//! Bản gốc ở `AuthService` dùng `HashMap` không giới hạn — chấp nhận được vì chỉ có người
//! dùng trên loopback. Endpoint MCP có thể công khai và khoá do **kẻ tấn công** điều
//! khiển, nên ở đây số khoá bị chặn trên [`MAX_TRACKED_KEYS`]: xoá trước entry đã hết cửa
//! sổ và không còn bị khoá; nếu vẫn đầy thì xoá entry có cửa sổ cũ nhất. Bộ nhớ luôn bị
//! chặn, và kẻ tấn công không dùng được việc đó để tắt hẳn giới hạn của khoá hợp lệ.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Cửa sổ quan sát mặc định — **giữ nguyên** giá trị của `POST /api/auth/login`.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);
/// Số lần thất bại trước khi khoá — **giữ nguyên** giá trị của `POST /api/auth/login`.
pub const DEFAULT_THRESHOLD: u32 = 5;
/// Trần thời gian khoá — **giữ nguyên** giá trị của `POST /api/auth/login`.
pub const DEFAULT_MAX_LOCK: Duration = Duration::from_secs(300);
/// Số lần bị ghi trước khi khoá tăng dần có tác dụng (`2^8 = 256` giây).
const MAX_BACKOFF_EXPONENT: u32 = 8;
/// Trần số khoá được theo dõi trong bộ nhớ (xem "Chặn tự cạn bộ nhớ" ở trên).
const MAX_TRACKED_KEYS: usize = 4096;

/// Trạng thái của **một** khoá.
///
/// `Default` vì `check()` tạo entry trước khi biết khoá đó có lỗi không — đúng như bản gốc
/// ở `AuthService`.
#[derive(Debug, Default, Clone, Copy)]
struct AttemptState {
    failures: u32,
    window_started: Option<Instant>,
    locked_until: Option<Instant>,
}

/// Bộ giới hạn tần suất theo khoá, dùng chung cho login và MCP server.
#[derive(Debug)]
pub struct RateLimiter {
    window: Duration,
    threshold: u32,
    max_lock: Duration,
    state: Mutex<HashMap<String, AttemptState>>,
}

impl Default for RateLimiter {
    /// Đúng tham số của `POST /api/auth/login` — hành vi đăng nhập không đổi.
    fn default() -> Self {
        Self::new(DEFAULT_THRESHOLD)
    }
}

impl RateLimiter {
    /// Bộ giới hạn với ngưỡng tuỳ chọn; cửa sổ và trần khoá giữ nguyên mặc định.
    #[must_use]
    pub fn new(threshold: u32) -> Self {
        Self::with_limits(threshold, DEFAULT_WINDOW, DEFAULT_MAX_LOCK)
    }

    /// Bộ giới hạn với cả ba tham số (dùng cho giới hạn lưu lượng của MCP).
    ///
    /// **Thứ tự gọi bắt buộc:** `check()` **trước**, rồi `record_*()`. Đây không phải quy
    /// ước lịch sự — nó là điều kiện đúng đắn: `check()` mới là nơi mở cửa sổ và đặt bộ đếm
    /// về 0, còn `record_*()` chỉ tăng bộ đếm. Gọi `record_*()` trước `check()` sẽ cộng
    /// dồn bộ đếm của cửa sổ *cũ* và có thể khoá sớm. Mọi caller trong repo (`AuthService`,
    /// `McpRateLimiter`) đều theo đúng thứ tự này.
    #[must_use]
    pub fn with_limits(threshold: u32, window: Duration, max_lock: Duration) -> Self {
        Self {
            window,
            threshold,
            max_lock,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Cửa sổ quan sát — để hiển thị/log.
    #[must_use]
    pub fn window(&self) -> Duration {
        self.window
    }

    /// Ngưỡng trước khi khoá.
    #[must_use]
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    /// Số khoá đang được theo dõi (chẩn đoán/test).
    #[must_use]
    pub fn tracked_keys(&self) -> usize {
        self.lock_state().len()
    }

    /// Mutex bị poison chỉ xảy ra khi một thread khác panics khi đang giữ khoá. Ta
    /// **không** panic (mục 0.8) và cũng **không** bỏ qua giới hạn: `recover` giữ nguyên
    /// dữ liệu đếm được, nên request kế tiếp vẫn bị khoá đúng như bình thường.
    fn lock_state(&self) -> MutexGuard<'_, HashMap<String, AttemptState>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Kiểm tra khoá có đang bị khoá không. `Ok(())` nghĩa là được phép đi tiếp.
    ///
    /// Cửa sổ được mở (và bộ đếm đặt về 0) ngay ở lần đầu sau khi hết cửa sổ cũ — đúng
    /// như bản gốc ở `AuthService::check_limit`.
    pub fn check(&self, key: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let mut map = self.lock_state();
        self.evict_if_full(&mut map, now);
        let state = map.entry(key.to_string()).or_default();
        if let Some(until) = state.locked_until
            && until > now
        {
            let wait = until
                .checked_duration_since(now)
                .map_or(Duration::from_secs(1), |left| {
                    Duration::from_secs(left.as_secs().saturating_add(1))
                });
            return Err(wait);
        }
        if state
            .window_started
            .is_none_or(|started| now.saturating_duration_since(started) >= self.window)
        {
            state.failures = 0;
            state.window_started = Some(now);
            state.locked_until = None;
        }
        Ok(())
    }

    /// Ghi nhận **một lần thất bại** (login: sai mật khẩu; MCP: token không hợp lệ).
    ///
    /// Chỉ gọi khi thật sự thất bại — traffic hợp lệ không được tính vào đây, nếu không
    /// một phiên coding dài sẽ tự khoá chính mình.
    pub fn record_failure(&self, key: &str) {
        self.bump(key);
    }

    /// Ghi nhận **một lần dùng** (MCP: mọi request qua `/mcp` đã vượt xác thực).
    ///
    /// Cùng thuật toán với [`Self::record_failure`], khác chỗ gọi. Giữ hai tên riêng để
    /// người đọc thấy ngay tại chỗ gọi rằng đang đếm lỗi hay đếm lưu lượng.
    pub fn record_request(&self, key: &str) {
        self.bump(key);
    }

    /// Xoá trạng thái của khoá sau khi dùng thành công (giống `clear_attempts`).
    ///
    /// Chỉ gọi khi khoá **vừa hợp lệ**: xoá sau mọi request thì kẻ tấn công có token
    /// đúng xen kẽ vài lần thất bại để lách được giới hạn.
    pub fn clear(&self, key: &str) {
        self.lock_state().remove(key);
    }

    /// Tăng bộ đếm của khoá và khoá tăng dần khi vượt ngưỡng.
    fn bump(&self, key: &str) {
        let now = Instant::now();
        let mut map = self.lock_state();
        self.evict_if_full(&mut map, now);
        let state = map.entry(key.to_string()).or_default();
        state.failures = state.failures.saturating_add(1);
        if state.failures >= self.threshold {
            let exponent = state
                .failures
                .saturating_sub(self.threshold)
                .min(MAX_BACKOFF_EXPONENT);
            let seconds = 1_u64 << exponent;
            state.locked_until = now.checked_add(Duration::from_secs(seconds).min(self.max_lock));
        }
    }

    /// Giữ số khoá trong bộ nhớ có trần (xem module docs).
    fn evict_if_full(&self, map: &mut HashMap<String, AttemptState>, now: Instant) {
        if map.len() < MAX_TRACKED_KEYS {
            return;
        }
        let window = self.window;
        map.retain(|_, state| {
            state.locked_until.is_some_and(|until| until > now)
                || state
                    .window_started
                    .is_some_and(|started| now.saturating_duration_since(started) < window)
        });
        if map.len() < MAX_TRACKED_KEYS {
            return;
        }
        // Vẫn đầy ⇒ xoá khoá có cửa sổ cũ nhất (đã hết tác dụng nhiều nhất).
        let oldest = map
            .iter()
            .filter_map(|(key, state)| {
                state
                    .window_started
                    .map(|started| (now.saturating_duration_since(started), key.clone()))
            })
            .min_by_key(|(elapsed, _)| *elapsed)
            .map(|(_, key)| key);
        if let Some(key) = oldest {
            map.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn allows_until_threshold_then_locks() {
        let limiter = RateLimiter::default();
        for _ in 0..DEFAULT_THRESHOLD {
            limiter.check("ip:1.2.3.4").unwrap();
            limiter.record_failure("ip:1.2.3.4");
        }
        // Lần thứ `threshold + 1` bị khoá.
        let wait = limiter.check("ip:1.2.3.4").unwrap_err();
        assert!(wait >= Duration::from_secs(1), "{wait:?}");
    }

    #[test]
    fn lockout_grows_then_caps() {
        let limiter = RateLimiter::with_limits(2, DEFAULT_WINDOW, DEFAULT_MAX_LOCK);
        let first = {
            for _ in 0..2 {
                limiter.check("ip:x").unwrap();
                limiter.record_failure("ip:x");
            }
            limiter.check("ip:x").unwrap_err()
        };
        let later = {
            for _ in 0..3 {
                limiter.check("ip:x").unwrap_err();
                limiter.record_failure("ip:x");
            }
            limiter.check("ip:x").unwrap_err()
        };
        assert!(later > first, "khoá phải tăng dần: {first:?} -> {later:?}");
        for _ in 0..40 {
            let _ = limiter.check("ip:x");
            limiter.record_failure("ip:x");
        }
        let capped = limiter.check("ip:x").unwrap_err();
        assert!(capped <= DEFAULT_MAX_LOCK, "khoá phải có trần: {capped:?}");
    }

    #[test]
    fn keys_are_independent_and_clear_resets() {
        let limiter = RateLimiter::new(2);
        for _ in 0..2 {
            limiter.check("token:a").unwrap();
            limiter.record_failure("token:a");
        }
        assert!(limiter.check("token:a").is_err());
        // Khoá khác không bị ảnh hưởng — nếu không, một IP bị khoá sẽ chặn cả server.
        assert!(limiter.check("token:b").is_ok());
        assert!(limiter.check("ip:9.9.9.9").is_ok());
        // `check()` tạo entry ngay cả khi khoá đó đang tốt (đúng như bản gốc ở
        // `AuthService::check_limit`), nên 3 khoá đang được theo dõi.
        assert_eq!(limiter.tracked_keys(), 3);
        // Xác thực thành công thì xoá đúng khoá đó, không đụng khoá khác.
        limiter.clear("token:a");
        assert_eq!(limiter.tracked_keys(), 2, "clear chỉ xoá đúng một khoá");
        assert!(limiter.check("token:a").is_ok());
    }

    #[test]
    fn volume_limit_uses_the_same_mechanism() {
        let limiter = RateLimiter::with_limits(3, DEFAULT_WINDOW, DEFAULT_MAX_LOCK);
        for _ in 0..3 {
            limiter.check("token:c").unwrap();
            limiter.record_request("token:c");
        }
        assert!(limiter.check("token:c").is_err());
    }

    #[test]
    fn window_resets_counter() {
        // Hết cửa sổ thì bộ đếm thất bại phải về 0 — nếu không, một IP dùng lâu sẽ bị
        // khoá vĩnh viễn chỉ vì vài lần gõ sai mật khẩu rải rác trong nhiều phút.
        // Thứ tự `check()` → `record_*()` là bắt buộc (xem `with_limits`).
        let limiter = RateLimiter::with_limits(2, Duration::from_millis(30), DEFAULT_MAX_LOCK);
        limiter.check("ip:w").unwrap();
        limiter.record_failure("ip:w");
        std::thread::sleep(Duration::from_millis(60));
        // `check()` mở cửa sổ mới ⇒ bộ đếm về 0, nên lần sai kế tiếp là lần thứ nhất
        // của cửa sổ đó và chưa vượt ngưỡng 2.
        limiter.check("ip:w").unwrap();
        limiter.record_failure("ip:w");
        assert!(
            limiter.check("ip:w").is_ok(),
            "hết cửa sổ thì bộ đếm phải về 0, không cộng dồn thành khoá"
        );
    }

    #[test]
    fn lock_outlives_the_window_on_purpose() {
        // Thời gian khoá **cố ý** độc lập với cửa sổ: ngưỡng 2 ⇒ khoá 1 giây, dù cửa
        // sổ chỉ 30ms. Đây là hành vi giống hệt `POST /api/auth/login` (bản gốc cũng
        // trả sớm khi `locked_until` còn hiệu lực, không chờ hết cửa sổ). Test này khoá
        // lại hành vi để đổi công thức backoff không vô tình làm yếu lớp chống dò.
        let limiter = RateLimiter::with_limits(2, Duration::from_millis(30), DEFAULT_MAX_LOCK);
        // check → record, check → record: hai lần sai ⇒ vượt ngưỡng 2 ⇒ khoá.
        for _ in 0..2 {
            limiter.check("ip:v").unwrap();
            limiter.record_failure("ip:v");
        }
        let wait = limiter.check("ip:v").unwrap_err();
        assert!(
            wait >= Duration::from_secs(1),
            "khoá phải dài hơn cửa sổ: {wait:?}"
        );
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            limiter.check("ip:v").is_err(),
            "cửa sổ hết không được xoá khoá đang có hiệu lực"
        );
    }

    #[test]
    fn tracked_keys_stay_bounded() {
        let limiter = RateLimiter::new(5);
        for index in 0..(MAX_TRACKED_KEYS * 2) {
            let key = format!("token:attacker-{index}");
            // `let _ =` vì `check` trả `Result`; ở test này ta cố tình bỏ qua kết quả —
            // mục tiêu là xem số khoá có vượt trần hay không.
            let _ = limiter.check(&key);
            limiter.record_failure(&key);
        }
        assert!(
            limiter.tracked_keys() <= MAX_TRACKED_KEYS,
            "bộ nhớ phải có trần: {}",
            limiter.tracked_keys()
        );
    }
}
