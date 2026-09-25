# Quyết định thiết kế (bổ sung/giải thích cho `agents.md`)

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
* **D3.4** **M14:** `rmcp 3.4.1` (Cargo range `3.4.0`, lockfile chốt 3.4.1), chỉ bật
  `default-features = false` + `["client", "transport-child-process"]`. API đã đối chiếu source
  crate đã tải: `TokioChildProcess::new`, `ServiceExt::serve`, `Peer::list_all_tools`,
  `RunningService::call_tool_once`, `CallToolRequestParams`; không dùng server/macros/HTTP của
  `rmcp`. `TokioChildProcess` tự kill/reap khi drop; BeanAgent vẫn gọi `close_with_timeout`
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

## 6. Quyết định riêng của M2 (providers)

* **D6.1** Mỗi lượt `LlmProvider::chat` phát đúng **một** HTTP request; retry/backoff nằm ở
  `beanagent_llm::retry::retry_with_backoff` (tối đa 3 lần, tôn trọng `Retry-After` giây,
  backoff 1s→2s→4s + jitter ≤ 25%) **bọc ngoài** closure request, không nằm trong provider.
  Lý do: test đo được số request qua wiremock, và M17 (streaming) không phải nhân bản logic.
* **D6.2** `reqwest` giữ backend mặc định rustls (0.13: rustls là default; **không** khai
  feature `rustls-tls` — feature này đã đổi tên ở 0.13, khai sai sẽ fail build). Chỉ thêm
  feature `json`. Timeout tổng 300s, connect 30s, User-Agent `BeanAgent/<version>`.
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

* **D7.1** Path jail đặt sau trait `WorkspaceFs` (`beanagent_tools::workspace`): M3 dùng
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
* **D7.5** `MemoryStore` (`beanagent-core::store`): in-memory; `history(session, before,
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
* **D8.2** **Một** thread riêng (`beanagent-memory-worker`) sở hữu `rusqlite::Connection`
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
  thật ở `BeanAgent chat`; CLI bọc nó trong `Arc` để chia sẻ với tool bộ nhớ.
* **D8.9** Context builder (mục 8.2) đặt ở `beanagent-core::context::build(store, config,
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
