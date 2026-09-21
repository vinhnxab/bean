# Quyết định thiết kế (bổ sung/giải thích cho `agents.md`)

File này ghi lại các điểm mà `agents.md` còn mơ hồ, mâu thuẫn hoặc thiếu, kèm quyết định
đã chốt và lý do. Người dùng đã uỷ quyền cho coding agent tự chốt các điểm này (2026-09-21).
Khi `agents.md` được cập nhật, mục tương ứng ở đây chuyển sang trạng thái "đã vào spec".

---

## 1. Kiến trúc Router / Run / RunIo (agents.md mục 6, 10)

* **D1.1 — Run thuộc Router.** `Router::submit()` trả `RunId` ngay và spawn task nền. Kênh
  (CLI/web/Telegram) **không** gọi `run_turn()`; kênh chỉ `submit` + hiển thị `RunEvent`.
* **D1.2 — `RunIo` do Router sở hữu, không mượn theo kết nối.** Trait là
  `RunIo: Send + Sync + 'static`; agent loop nhận `Arc<dyn RunIo>`. Lý do: mục 10 yêu cầu run
  sống qua việc rớt WebSocket, nên không thể mượn `&dyn RunIo` gắn với kết nối.
  * M3: CLI tự implement `RunIo` (in ra terminal, hỏi y/n).
  * M8: Router cung cấp `RouterIo` cùng trait — `on_*` phát `RunEvent` lên broadcast,
    `confirm()` sinh `confirm_id`, phát `ConfirmRequest`, chờ `oneshot` với `timeout_seconds`,
    hết hạn ⇒ `DENY`.
* **D1.3 — Một enum sự kiện duy nhất.** `RunEvent` (định nghĩa ở `BeanAgent-types`) luôn mang
  `session_id`, `run_id`, và `message_id` khi có. `BeanAgent-web` chỉ **map** 1-1 sang `ServerMsg`
  (thêm trường `type` cho TS); không định nghĩa hai enum song song gần giống nhau.
* **D1.4 — `Incoming` có `session_id: Option<SessionId>`.** Web: mỗi hội thoại = 1 session = 1
  `chat_id` (uuid). `session_id = None` ⇒ resolve theo `(channel, chat_id)` như mục 8.1
  (CLI/Telegram). Nếu có `session_id`, Router phải kiểm tra session đó thuộc đúng
  `channel`/`chat_id`/`user_id` trước khi dùng (chống IDOR).
* **D1.5 — Web là một `Channel`.** `BeanAgent-web::WebChannel`: `run()` = chạy axum server và chờ
  `shutdown`; `send(chat_id, Outbound)` = broadcast `Notification` tới mọi kết nối WS.
  Router giữ registry `HashMap<&'static str, Arc<dyn Channel>>` (`register_channel`).
* **D1.6 — `Outbound` có kiểu** `{ session_id, message_id, text, kind }`. Kênh đẩy (Telegram)
  dùng bảng `outbox` để thử lại; **web không cần retry** vì tin đã nằm trong DB và UI nạp lại
  bằng REST — ghi rõ như vậy để tránh implement retry vô nghĩa.
* **D1.7 — "Cho phép trong phiên"** lưu **trong RAM theo `(session_id, tool_name)`**, không ghi
  DB; xoá khi `/new`, khi restart, và bị veto trong lượt nếu đã đọc `<untrusted_content>` (15.4).
* **D1.8 — Ngưỡng output tool (3 mức khác nhau)**: lưu DB tối đa **1 MiB** (quá thì cắt + ghi chú),
  gửi model tối đa **20.000 ký tự** tại ranh giới UTF-8, preview UI **2.000 ký tự**.
  Nhờ vậy `GET /api/messages/:id` thật sự có nội dung đầy đủ để hiển thị.

## 2. Bổ sung schema SQLite (agents.md mục 8.1)

* **D2.1** Thêm `usage(day TEXT PRIMARY KEY, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL)`
  — mục 15.9 yêu cầu ngân sách token/ngày nhưng schema cũ không có chỗ đếm.
* **D2.2** `outbox` thêm `next_attempt_at TEXT NOT NULL`, `last_error TEXT NOT NULL DEFAULT ''`
  (index theo `next_attempt_at`) để "thử lại có backoff" khả thi.
* **D2.3** `scheduled_tasks` thêm `session_id INTEGER REFERENCES sessions(id)`, `created_at`,
  `last_run_at`, `last_status` (mục 14 nói "chạy agent trong session của task").
* **D2.4** `web_sessions` thêm `user_id TEXT NOT NULL DEFAULT 'web:admin'` để audit/`/api/auth/me`
  có định danh (v1 một người dùng).
* **D2.5** `messages.seq` do **một writer duy nhất** cấp (`MAX(seq)+1` trong cùng transaction).
* **D2.6** FTS5: dùng `unicode61 remove_diacritics 2` cho `messages_fts`/`memories_fts`
  (tìm được cả khi người dùng gõ không dấu); phải có đủ 3 trigger `ai/ad/au` và lệnh
  `INSERT INTO <fts>(<fts>) VALUES('rebuild')` dùng khi migration/restore.

## 3. Lựa chọn thư viện (agents.md mục 3.1, 3.2) — đã kiểm tra docs/source ngày 2026-09-21

* **D3.1** `reqwest 0.13`: `rustls` là backend **mặc định** (feature TLS tên là `rustls`, không còn
  `rustls-tls`); `query`/`form` không còn bật mặc định ⇒ khai báo tường minh `features = ["json"]`
  (+ `stream` khi làm M17).
* **D3.2** `argon2 0.6` (API khác 0.5): dùng `Argon2::default().hash_password(pw)?` +
  `PasswordHash::new(&phc)`; feature mặc định đã gồm `password-hash` + `getrandom`.
* **D3.3** `teloxide 0.17`: **tắt default features** (`native-tls`) và bật
  `["rustls", "rustls-native-roots", "macros", "throttle"]` để không kéo OpenSSL.
* **D3.4** `rmcp 3.4` (`client` + `transport-child-process`), MSRV 1.88; lưu ý có migration guide
  2.x→3.x khi làm M14.
* **D3.5** `tower-http 0.7` (+ `csrf` layer làm lớp phụ cho mục 15.7, và `services::fs::Backend`
  để phục vụ asset nhúng ở M9).
* **D3.6** DB: `tokio-rusqlite 0.8` (forward feature `bundled` sang `rusqlite 0.40`).
  `rusqlite` `bundled` **đã có FTS5** (`libsqlite3-sys` bật `SQLITE_ENABLE_FTS5`) — không cần
  feature riêng.
* **D3.7** Cron: `croner 4` (có `.describe()` cho UI màn Tác vụ).
* **D3.8** Frontmatter của skill: **tự parse 2 trường** `name`/`description` (không dùng `serde_yaml`
  — repo dtolnay/serde-yaml đã archived; `serde_yaml_ng`/`serde_norway` là phương án dự phòng).
* **D3.9** Cấu hình dùng `toml 1.1` (đã là major v1).
* **D3.10** Web: TypeScript **7** (native, vẫn cung cấp `tsc`), Vite **8** (Rolldown/Oxc;
  `build.rolldownOptions` thay `build.rollupOptions`), React 19.3, react-router **8**,
  Vitest **5** (`test.projects` thay "workspace"), Tailwind **4.3** (`@tailwindcss/vite`,
  không có `tailwind.config.js`), shadcn CLI 4.
* **D3.11** Highlight code: **highlight.js** (class-based, không sinh inline style) thay vì shiki
  (sinh `style="…"` inline) — M10 dùng.
* **D3.12** i18n: **từ điển tự viết** (`web/src/i18n/{vi,en}.ts`), không dùng react-i18next.
* **D3.13** pnpm **11**: `onlyBuiltDependencies` đã bị xoá; dùng map **`allowBuilds`** trong
  `web/pnpm-workspace.yaml`, `strictDepBuilds: true` (mặc định) ⇒ dependency có build script phải
  được duyệt tường minh.

## 4. Web server / an toàn UI (agents.md mục 11, 12, 15.7)

* **D4.1** CSP giữ `default-src 'self'`, nhưng cho phép `style-src 'self' 'unsafe-inline'`
  (Tailwind v4/HMR và một số component sinh style inline; không nới thì UI không render đúng).
  `script-src 'self'` không nới; `img-src 'self' data:`; ghi comment nêu lý do trong code M9.
* **D4.2** CSRF: yêu cầu `Content-Type: application/json` + kiểm tra `Origin`/`Host` với mọi request
  thay đổi dữ liệu, **thêm** `tower_http::csrf::CsrfLayer` làm lớp phụ.
* **D4.3** Giới hạn tần suất đăng nhập theo **peer IP** (`ConnectInfo<SocketAddr>`); chỉ tin
  `X-Forwarded-For`/`X-Real-IP` khi `web.trust_proxy = true`.
* **D4.4** `ClientMsg::Start.text` tối đa 64 KB; vượt ⇒ `Error{code:"payload_too_large"}`.
* **D4.5** UI khử trùng theo `message_id`/`run_id`: lịch sử luôn lấy từ REST, `Sync` để khôi phục
  run/confirm đang chạy; event đến sau khi REST đã nạp thì bỏ qua theo `message_id`.
* **D4.6** `serve` từ chối bật web nếu chưa có `data.dir/auth.toml` (đặt bằng
  `BeanAgent auth set-password`: hash `argon2id`, file quyền `0600`).

## 5. Nhỏ nhưng cần chốt để không tự suy diễn

* **D5.1** Tên file spec trong repo này là `agents.md` (chữ thường).
* **D5.2** `make types` ở M1 là placeholder: chưa có kiểu API nào; từ M9 target này chạy
  `cargo test --workspace export_bindings` với `TS_RS_EXPORT_DIR=web/src/api/generated`
  (đặt trong `.cargo/config.toml`), và `make check-rust` so `git diff --exit-code` trên thư mục đó.
* **D5.3** `cargo build --no-default-features` chạy ở gốc workspace nhờ
  `default-members = ["crates/BeanAgent"]`.
* **D5.4** `run_turn()` trả `RunOutcome { text, ended: EndReason }` với
  `EndReason::{Final, MaxSteps, Cancelled, BudgetExceeded, LoopGuard}` (mục 6 chỉ trả `String`,
  không phân biệt được lý do dừng để UI/Telegram thông báo cho đúng).
* **D5.5** Chống lặp so sánh **JSON đã chuẩn hoá** (sort khoá) của `(tool, args)`.
* **D5.6** `allowed_users` bị kiểm tra **trước** khi tạo session/ghi message; chỉ log warn.
* **D5.7** `/model` chỉ đổi trong runtime, áp dụng cho run kế tiếp, không ghi vào `BeanAgent.toml`.
* **D5.8** Hai chỗ dùng múi giờ khác nhau: `agent.timezone` cho **parse cron/hiển thị phía server**;
  UI luôn hiển thị theo múi giờ trình duyệt.
* **D5.9** `Risk` được định nghĩa ở `BeanAgent-types` (mục 7.1 đặt ở `BeanAgent-tools`) để
  `tools`, `core` và `web` dùng chung một kiểu; `BeanAgent-tools` re-export lại.
* **D5.10** `BeanAgent.toml` dùng `#[serde(deny_unknown_fields)]` để bắt lỗi gõ sai khoá; khoá thêm
  ngoài mục 18 (`web.trust_proxy`) đều có `#[serde(default)]` và nằm trong file mẫu dạng comment.
* **D5.11** `chat` chạy được **không cần** `BeanAgent.toml` (dùng giá trị mặc định, có cảnh báo)
  để `cargo run -p BeanAgent -- chat` là lệnh smoke test của M1.
* **D5.12** Mọi crate: `#![forbid(unsafe_code)]`; clippy `unwrap_used`/`expect_used`/`panic`/`todo`
  = `deny` ở `[workspace.lints]`; file test được phép dùng `unwrap`/`panic` qua `#![allow(...)]`
  ở đầu file.
* **D5.13** Tên **package** của các crate thư viện là kebab-case chữ thường (`beanagent-types`,
  `beanagent-llm`, …) và thư mục cũng vậy (`crates/beanagent-types`), vì Rust dùng tên package
  làm tên crate trong code: `BeanAgent-types` sẽ buộc phải viết `BeanAgent_types::…` (không
  idiomatic). Package của binary vẫn là `BeanAgent` (để `cargo run -p BeanAgent -- chat` và tên
  file binary là `BeanAgent`), và `[[bin]] name = "BeanAgent"` được khai báo tường minh.
  Đây là sai khác nhỏ so với sơ đồ mục 4 của `agents.md` (chỉ khác chữ hoa/thường).
