//! # BeanAgent-core
//!
//! Lõi agent: vòng lặp, context, Router, `trait Channel`, scheduler, learning loop
//! (agents.md mục 6, 10, 14, 17).
//!
//! * **M3**: `agent::run_turn` (`max_steps`, lỗi tool ⇒ `is_error`, timeout từng tool, cắt output
//!   tại ranh giới UTF-8, chống lặp, `CancellationToken`), `trait RunIo` (do Router sở hữu —
//!   `docs/decisions.md` D1.2), `trait Store` bản in-memory, `agent::prompt` theo mục 19.
//! * **M8**: `Router` (hàng đợi theo session, `submit`/`events`/`resolve_confirm`/`cancel`/`notify`,
//!   `RunEvent`, `Outbound`) + slash command xử lý trong lõi, `allowed_users`.
//! * **M13**: `scheduler` (tick 30 giây, `trait Clock`, croner + chrono-tz, outbox retry).
//! * **M15**: learning loop (reflection sinh skill nháp, chỉ kích hoạt khi người dùng duyệt).
//!
//! Trách nhiệm của crate này là **quyền quyết định**: run thuộc Router chứ không thuộc kết nối,
//! confirm do Router cấp `confirm_id` và phân giải, huỷ chỉ khi có yêu cầu tường minh.
#![forbid(unsafe_code)]
