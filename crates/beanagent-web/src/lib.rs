//! # BeanAgent-web
//!
//! Web server bằng `axum`: xác thực, REST, WebSocket, phục vụ UI nhúng (agents.md mục 11, 15.7).
//!
//! * **M9**: `auth` (argon2id + cookie phiên, chỉ lưu hash token), middleware `Origin`/`Host`
//!   và `Content-Type: application/json` cho request thay đổi dữ liệu (kèm `tower_http::csrf`),
//!   header bảo mật (CSP `style-src 'self' 'unsafe-inline'` — lý do ở `docs/decisions.md` D4.1),
//!   REST theo mục 11.1, WebSocket `/api/ws` kiểm tra Origin + cookie **trước** khi nâng cấp,
//!   `Sync` sau khi nối lại, và map `RunEvent` → `ServerMsg`.
//! * `WebChannel` implement `trait Channel` của `BeanAgent-core` (`run()` = chạy axum,
//!   `send()` = broadcast `Notification` tới mọi tab — `docs/decisions.md` D1.5).
//! * Phục vụ asset: `rust-embed` sau feature `ui`, `#[allow_missing]` để build không cần
//!   `web/dist`; SPA fallback **không** được che lỗi 404 của `/api/*`.
#![forbid(unsafe_code)]
