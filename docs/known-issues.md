# Ghi chú dự án — quyết định của chủ dự án & điểm yếu đã biết

> **Ghi chú đổi tên (2026-09-28).** Đổi tên `BeanAgent`/`beanagent-*` → `bean`/`bean-*` trên
> toàn repo (xem `docs/decisions.md`, mục 19 — D18.x). Tên crate, tên lệnh và tên đường dẫn
> trong file này đã cập nhật sang tên mới. Tag `pre-rename-beanagent` giữ trạng thái commit
> gốc mà các ghi chú này mô tả, dùng khi cần tái hiện đúng môi trường cũ.

File này dành cho việc **nhớ lại quyết định đã chốt** và **ghi nhận điểm yếu còn tồn đọng**,
để milestone sau xử lý tiếp thay vì phát hiện lại từ đầu.

* Lý do kỹ thuật chi tiết của từng quyết định: `docs/decisions.md` (M5 = mục 8, `D8.1`–`D8.10`).
* Yêu cầu gốc (đừng sửa file này để đổi phạm vi): `AGENTS.md`, bản prompt theo milestone: `PROMPTS.md`.

Cập nhật lần cuối: 2026-09-28 (M26 — tool browser nội bộ nói thẳng CDP, thay chrome-devtools-mcp; thêm K25).

---

## 1. Quyết định của chủ dự án — đã chốt, không tự ý đổi

| # | Ngày | Quyết định | Ghi ở đâu |
|---|------|-----------|-----------|
| Q1 | 2026-09-24 | **Bỏ nhân bản system prompt.** System chỉ gửi ở `ChatRequest.system`; không tạo thêm message `User` chứa lại system prompt (M3 từng làm vậy → tốn token gấp đôi và dễ bị model hiểu nhầm là lời người dùng). Đã được duyệt sau khi báo cáo đánh đổi token; có test hồi quy chốt hành vi. | `D8.10`, test `system_prompt_is_sent_once_via_system_field` |
| Q2 | 2026-09-23 | **Code phải là file Rust thật, không hack bằng `include!("/tmp/…")`.** Đã loại bỏ cách ghép file tạm trong `store.rs`; mọi thứ phải qua `cargo fmt`, `clippy`, review được. | commit `c22bde5` |
| Q3 | 2026-09-23 | **`make check` (Rust + web) là cổng bắt buộc** trước khi báo "xong milestone"; commit riêng sau mỗi milestone theo `AGENTS.md` mục 0.5. | `Makefile`, lịch sử git |
| Q4 | 2026-09-23 | **Bám `PROMPTS.md`/mục 8.2 khi hiểu phạm vi**: context builder = system + `MEMORY.md`/`USER.md` + `sessions.summary` + lịch sử vừa ngân sách token — không phải chỉ cắt history. (Lần đầu làm thiếu phần này, đã bổ sung sau khi đối chiếu lại checklist M5.) | `crates/bean-core/src/context.rs`, `D8.9` |
| Q5 | 2026-09-24 | **Ghi quyết định + điểm yếu vào repo** để xử lý ở milestone sau (thay vì chỉ nói trong chat). | file này |

### Nguyên tắc kỹ thuật đã chốt (đừng phá khi tối ưu)
* **Một writer duy nhất cho SQLite**: `rusqlite` là API blocking nên mọi truy cập đi qua worker
  thread `bean-memory-worker`; không bao giờ gọi DB trực tiếp trong async (`D8.2`).
* **Không bao giờ tách cặp `assistant(tool_calls)` / `tool` result** khi cắt lịch sử
  (`mục 8.3, 22.1`); proptest trong `safe_cut.rs` phải luôn xanh.
* **Ước lượng token = `chars/4`**; ngân sách ở `context_budget_tokens` tính **riêng cho lịch sử**
  (không trừ system prompt) và compaction dùng **cùng** công thức. Nếu đổi thì phải sửa cả hai
  chỗ cùng lúc, nếu không context và compaction lệch nhau (`D8.6`, `D8.9`).
* **Query của model là dữ liệu không tin cậy** — luôn làm sạch trước khi đưa vào SQL/FTS5.

---

## 2. Điểm yếu đã biết — việc cần khắc phục

Mức độ: **cao** = có thể sai lệch về hành vi/an toàn · **trung bình** = chất lượng/độ bền ·
**thấp** = ghi chú để không ai "sửa nhầm".

| # | Vấn đề | Vì sao là vấn đề | Hướng khắc phục | Mức | Mốc gợi ý |
|---|--------|------------------|------------------|------|-----------|
| S1 | ✅ **ĐÃ XỬ LÝ (2026-09-26)** — *Prompt injection qua tool đọc file/lệnh.* `read_file`, `grep`, `glob`, `list_dir` và output `run_shell` trả văn bản thô, không bọc `<untrusted_content>` và không bật `untrusted_seen` ⇒ lớp phòng thủ "cho phép trong phiên" (mục 15.4) mất tác dụng: đọc một file độc là đủ để `write_file`/`run_shell` chạy **không hỏi lại**. | Đã khai thác được và chứng minh bằng test dùng **tool thật**. Chi tiết: `docs/security-review.md` mục 2. | **Đã vá:** (1) bọc `<untrusted_content>` cho cả 5 tool, tái dùng `untrusted::wrap_bounded`; (2) bật `untrusted_seen`; (3) cắt vẫn ở ranh giới UTF-8; (4) thêm `Tool::marks_untrusted()` — khai báo tường minh, agent loop bật cờ theo khai báo (giữ `contains_untrusted_block` làm lưới an toàn thứ hai); (5) `list_dir`/`glob` cũng bật cờ vì tên file là dữ liệu kẻ tấn công kiểm soát. Test: `core/tests/untrusted_file.rs` + `security/tests/untrusted_tools.rs` (gồm test hồi quy toàn registry). Thiết kế: `D9.1`. | ~~cao~~ → **đã đóng** | Xong — **K1 cũng đã đóng 2026-09-27** |
| S2 | ✅ **ĐÃ XỬ LÝ (2026-09-27)** — *Terminal escape injection qua CLI.* `chat.rs` in thẳng `output_preview` (output của tool) ra terminal bằng `println!`; chuỗi ANSI/OSC do kẻ tấn công kiểm soát (file đọc được, output lệnh trong container) có thể vẽ lại màn hình, **che giấu prompt xác nhận** để người dùng bấm nhầm, hoặc dùng OSC 52 cài sẵn clipboard. | Không cần chiếm quyền, chỉ cần nạp được nội dung độc vào workspace/container. Tác động lên **người dùng** (UX + clipboard), không lên quyền hệ thống. Chi tiết: `docs/security-review.md` mục 3.2. | **Đã vá:** (1) hàm dùng chung `bean_tools::text::strip_terminal_escapes` — nuốt **trọn chuỗi escape** (CSI/OSC/DCS, 7-bit và 8-bit), giữ `\n`/`\t` và mọi chữ thường; (2) CLI lấy danh sách tool untrusted từ **chính khai báo `marks_untrusted()`** (D9.1) lúc khởi động ⇒ không có danh sách tool thứ hai phải đồng bộ; (3) `render_event` nhận `&mut dyn Write` để assert trên buffer thật. Tool **không** untrusted (`write_file`, D9.5) giữ nguyên hành vi cũ — không lọc thừa. Test: `chat.rs` (4 test) + `text.rs` (4 test). Thiết kế: `D9.8`. | ~~thấp–TB~~ → **đã đóng** | Xong. Lọc ở CLI; web/Telegram render bằng React/plain text nên không dính |
| K1 | ✅ **ĐÃ XỬ LÝ (2026-09-27)** — *Prompt injection qua `sessions.summary`.* Summary do LLM sinh từ nội dung người dùng + kết quả tool (có thể chứa nội dung web/MCP không tin cậy) rồi được chèn thẳng vào **system prompt** ⇒ dữ liệu không tin cậy biến thành chỉ dẫn cấp hệ thống. | Xung đột trực tiếp với mục 15.4; là đường leo đặc quyền từ *dữ liệu* sang *chỉ dẫn*. **Cùng lớp lỗi với S1 nhưng đi đường khác** — S1 đã vá 2026-09-26, K1 vá 2026-09-27. | **Đã vá (phương án (a)):** (1) bọc summary trong `<untrusted_content>` bằng lại `untrusted::wrap_bounded` — không viết thuật toán bọc mới; (2) **bật `untrusted_seen` khi context có summary** qua cờ `TurnContext::summary_present` ⇒ mọi tool `Confirm` trở lên hỏi lại dù lượt đó không đọc nội dung ngoài lõi nào; (3) thẻ đóng giả trong summary bị escape như mọi tool khác. **Đánh đổi đã được duyệt:** phiên đã compact mất "cho phép trong phiên" cho `Confirm`/`Dangerous` tới hết phiên. Test: `core/tests/untrusted_summary.rs` (3 test) + `context.rs` (3 test hồi quy). Thiết kế: `D9.7`. | ~~cao~~ → **đã đóng** | Xong. Xem `K1-followup` bên dưới |
| K1-followup | **Cân nhắc theo dõi `summary_untrusted`** — chỉ bật cờ khi summary thực sự tổng hợp từ nội dung đã untrusted, thay vì bật cho *mọi* phiên đã compact. | Sau D9.7, mọi lượt của phiên đã compact đều phải bấm "Cho phép" cho từng hành động `Confirm`; ở phiên dài, thao tác lặp nhiều sẽ thành mỏi. | Ghi cờ lúc compaction (kiểm tra message nguồn có chứa `<untrusted_content>` không) vào cột mới + migration `user_version`; `context.rs` chỉ bật cờ khi cờ đó đúng. | thấp | **Chưa làm ngay** — chưa có bằng chứng người dùng thấy mỏi tay đáng kể; cần dữ liệu dùng thật. Nằm ngoài phạm vi lượt vá K1. |
| K2 | `context::build` đọc **toàn bộ** lịch sử ở mỗi bước để cắt theo ngân sách. | O(n) mỗi lượt gọi LLM; sẽ chậm dần khi phiên dài. | Đẩy `limit` xuống SQL (đếm ngược từ `seq` mới nhất) hoặc cache theo `session + seq_max`. | trung bình | M8 hoặc M16 |
| K3 | `SqliteStore::request` **không có timeout**. | Worker kẹt ⇒ mọi lời gọi store treo mãi, run không kết thúc. | Bọc `tokio::time::timeout`, trả `StoreError::Internal` rõ ràng; cân nhắc hàng đợi ưu tiên cho `append`. | trung bình | M8/M16 |
| K4 | Mọi lệnh DB xếp hàng trên **một** worker. | Một truy vấn FTS nặng chặn cả `append` (ghi message ngay khi phát sinh — mục 6). | Tách lệnh chỉ đọc sang connection riêng (WAL chịu nhiều reader), hoặc giới hạn `MEMORY_SEARCH_LIMIT` theo ngân sách thời gian. | trung bình | M16 |
| K5 | `memory_search` trả về **cả message của chính lượt đang chạy**. | `text_for_search` chứa cả đối số tool, nên lượt `memory_search` trước đó thường đứng đầu kết quả (thấy rõ khi chạy thật) — gây nhiễu cho model. | Loại message thuộc lượt hiện tại, hoặc chấm điểm riêng cho "hành động tool" so với "lời thoại". | trung bình | M6/M15 |
| K6 | `Store::clear` xoá message nhưng **không xoá `sessions.summary`**. | Summary vẫn mô tả lịch sử đã mất ⇒ context sai lệch. Hai bản cài đặt phải giữ cùng ngữ nghĩa. | Xoá (hoặc đánh dấu cũ) summary trong cả `SqliteStore` lẫn `MemoryStore`, kèm test hành vi chung. | trung bình | M8/M9 (API xoá session) |
| K7 | Hai cài đặt trait `Store` có thể **trôi lệch ngữ nghĩa**. | `MemoryStore` chỉ dùng cho test nhưng phải giống hệt `SqliteStore`; đã phải nới vài chỗ (ví dụ `append` tự tạo phiên) vì test M3 ghi thẳng `SessionId::new(1)`. | Một bộ test hành vi chạy cho **cả hai**; hoặc bỏ `MemoryStore` khi M8+ dùng SQLite thật. | trung bình | M8 |
| K8 | Xếp hạng BM25 chỉ tương đối **trong từng nguồn**. | Hai nguồn luôn có đỉnh `1.0` nên thứ tự giữa `memories` và `messages` là quy ước (ghi nhớ đứng trước), không phải điểm số thật. | Nếu cần trộn thật sự: công thức chuẩn hoá chung, hoặc `rrf` (reciprocal rank fusion) khi có nhiều nguồn. | thấp | M15 |
| K9 | `sanitize_fts_query` chỉ giữ chữ–số/`_`. | Từ khoá có ký hiệu bị bóp méo (`C++` → `C`, `rust-lang` → `rustlang`); luôn là AND, không có `OR`/`NEAR`. | Hoặc ghi rõ hạn chế trong description của tool, hoặc hỗ trợ cú pháp an toàn hơn (AND tường minh + bỏ ký tự thay vì nối liền). | thấp | M6/M7 |
| K10 | `messages.seq` tăng vô hạn, không reset sau compaction. | **Không phải lỗi** (seq là khoá tăng dần nên `before_seq` phân trang vẫn đúng) — ghi chú để không ai "sửa" thành index hay reset rồi làm hỏng phân trang. | — | thấp | — |
| K11 | Thiếu **E2E bền vững cho M5** trong `tests/e2e/`. | M5 mới kiểm chứng bằng script tạm trong `/tmp`: `memory_save` ở tiến trình 1, `memory_search` ở tiến trình 2, cộng mock HTTP server để dump payload. | Đóng gói thành fixture để `make e2e` (M16) dùng lại; nhớ `/tmp` của môi trường dev có thể bị dọn giữa chừng. | trung bình | M16 |
| K12 | Chưa kiểm chứng với **provider thật**. | Compaction + ngân sách token mới chạy với `FakeProvider`; môi trường build không có API key. Điều kiện "hội thoại dài không lỗi API" của M5 vì vậy mới đúng ở mức logic. | Một lượt thật (Anthropic hoặc OpenAI-compat) với hội thoại đủ dài để kích hoạt compaction, hoặc đưa vào `make e2e`. | trung bình | bất kỳ lúc nào có key; M16 |
| K13 | `/new` hiện chỉ có ở `chat.rs` (adapter). | Xử lý slash command thuộc lõi sẽ là M8 (Router); hiện chưa trùng lặp logic. | Khi M8 có Router: chuyển `/new`, `/stop`… vào lõi, adapter chỉ đọc dòng. | thấp | M8 |
| K14 | Chưa có API/UI đọc-ghi `MEMORY.md`/`USER.md`. | Trong prompt mục 19 và mục 8.4 có nhắc, nhưng thiết kế đặt ở M9 (REST) + M11 (UI). | Làm đúng milestone của nó, đừng kéo sớm. | thấp | M9/M11 |
| K15 | **Chưa smoke test Telegram với bot thật.** | M12 đã kiểm chứng adapter bằng `MockTransport` và Router thật, nhưng môi trường triển khai chưa có `TELEGRAM_BOT_TOKEN`; chưa xác minh long polling, typing, callback/approval, `/stop` và hành vi cùng web trên một instance thật. | Sau khi hoàn thành các milestone: dựng môi trường Telegram riêng, đặt token qua biến môi trường (không commit/ghi log), thêm đúng `telegram:<user_id>` vào `agent.allowed_users`, chạy `serve`; kiểm tra chat thường, tool cần xác nhận, Dangerous không có nút session, callback người lạ bị bỏ qua, `/stop`, reconnect và web đồng thời. Nếu cần, bổ sung checklist/e2e opt-in; không làm `make check` phụ thuộc secret. | trung bình | Sau tất cả milestone (M16+ / trước phát hành) |
| K16 | **Chưa có tín hiệu “người dùng phàn nàn” cho learning loop.** | M15 coi run `Final` và không có lỗi trong chính lượt đó là tín hiệu chưa phàn nàn; phản hồi sửa sai ở lượt sau không thể chặn một đề xuất vừa tạo. | Thêm feedback/rejection ngữ nghĩa rõ ràng (ví dụ phản hồi phủ nhận workflow) hoặc lưu reflection candidate ở trạng thái chờ rồi chỉ tạo draft sau khi lượt kế tiếp xác nhận không có phản hồi tiêu cực; test bằng `FakeProvider`. | trung bình | M16/backlog learning |
| K17 | **Reflection M15 mới chỉ được kiểm chứng bằng `FakeProvider`.** | Chưa xác minh model thật luôn trả đúng JSON schema, không gọi tool khi không được cấp, chấp nhận fenced JSON, và tạo SKILL.md hợp lệ trên Anthropic/OpenAI-compat/Ollama. | Chạy một kịch bản opt-in có provider thật hoặc mock HTTP tham gia xác thực wire format; kiểm tra output malformed, fenced JSON, body rỗng, description quá dài và update nhầm skill chưa load. Không đưa API key vào repo. | trung bình | Khi có provider/key; M16 hardening |
| K18 | **Chưa có E2E M15 xuyên qua binary/server thật.** | Unit/integration test đã chứng minh learning, REST và Telegram adapter, nhưng chưa có `make e2e` đi hết luồng: task ≥5 tool call → notification → xem draft/diff → duyệt → run kế tiếp thấy skill mới. | Bổ sung fake scenario không cần network vào `tests/e2e/`, chạy `serve --fake-llm`; kiểm tra restart vẫn giữ rate limit và loader không thấy skill trước khi duyệt. | trung bình | M16 (`make e2e`) |
| K19 | **Quyết định draft chưa có audit/lịch sử bền.** | Draft được duyệt hoặc bỏ rồi xóa khỏi `_drafts`; hiện chỉ có ID/status trả về request, chưa lưu actor, thời điểm, nội dung đã duyệt và lý do bỏ trong audit JSONL hoặc lịch sử riêng. | Ghi sự kiện tạo/duyệt/bỏ với actor đã xác thực vào audit; cân nhắc archive metadata sau quyết định, có retention policy và cách xem/xóa từ UI. Không log secret hoặc toàn bộ output tool. | trung bình | M16/security hardening |
| K20 | **Draft phát sinh từ scheduler không có nút inline Telegram.** | Scheduler không gắn với user Telegram cụ thể; adapter cố ý gửi notification dạng text để không gán callback cho sai user, nên phải dùng `/approve <id>` hoặc duyệt qua web. | Quy định chính sách rõ: chỉ tạo learning proposal từ interactive run, hoặc gửi scheduler draft về một chat quản trị được cấu hình và yêu cầu xác nhận từ user có quyền trong chat đó. | thấp | Backlog sau M15 |
| K21 | **Future-incompatibility warning từ `proc-macro-error2 v2.0.1`.** | `cargo report future-incompatibilities --id 1` báo `E0365` vì crate re-export `proc_macro` bằng `pub use proc_macro;`; Rust hiện tại chỉ cảnh báo nhưng tương lai có thể thành hard error. Dependency là gián tiếp: `teloxide 0.17.0` → `aquamarine 0.6.0` → `proc-macro-error2 2.0.1`, chỉ dùng lúc build vì là proc-macro, không phải lỗ hổng runtime; `make check` vẫn xanh. | Chờ upstream sửa hoặc cập nhật `aquamarine`/`teloxide` khi có bản tương thích; không tự patch dependency chỉ để im warning. Sau khi nâng Rust/dependency, chạy lại `cargo report future-incompatibilities`. | thấp | Backlog khi Rust/CI nâng version |
| K22 | **`docker_timeout_kills_container` flaky khi chạy song song.** | Test spawn container thật rồi đợi `docker kill`; khi `cargo test` chạy nhiều suite cùng lúc, container kế vẫn ở trạng thái `Created` nên assert "container mồ côi còn sống" fail. Chạy riêng (`cargo test -p bean-security --test sandbox_docker`) thì PASS, và `cargo test --workspace` chạy lại cũng xanh — nên là **race của test/máy**, không phải hồi quy logic sandbox. Thêm nữa máy dev không có sẵn image `bean-sandbox:latest` (phải `make build-sandbox`/docker build). | Serialize các test docker (một suite, chạy tuần tự `--test-threads=1`) hoặc poll container thay vì đo trạng thái ngay; CI nên build image trước và chạy phần docker ở job riêng. | trung bình | Khi thêm CI |
| K25 | **Lớp kiểm tra URL của tool browser còn sót khoảng hở DNS rebinding.** | `bean-browser::guard` **không phân giải DNS** — nó chỉ chặn host là IP literal nằm trong dải nội bộ. Một tên miền **ngoài** `allowed_origins` vẫn có thể trỏ về `127.0.0.1` hay `169.254.169.254` vào lúc Chromium thực sự kết nối, đi vòng qua lớp kiểm. Đây là hệ quả trực tiếp của việc **tách lớp kiểm riêng** cho domain browser thay vì sửa `bean_security::ssrf` dùng chung (sửa chung sẽ phá bảo đảm của `web_fetch`, vốn dùng resolver tuỳ biến lọc IP ngay lúc kết nối). Lưu ý: khe này **không** do so khớp wildcard — `origin.rs` khớp theo ranh giới label và đã có test riêng cho `dev.internal.attacker.com`. | Nếu cần đóng: cho `guard` dùng resolver tuỳ biến như `ssrf` để kiểm IP *đã phân giải*, hoặc bắt buộc `allowed_origins` chỉ chứa hostname/IP đã xác minh của môi trường test. **Hiện không làm** trong lượt này** vì rủi ro bị chặn bằng hai lớp khác: origin đó luôn `Dangerous` (không session-wide, mỗi lần đều phải bấm Duyệt) và nội dung trang vẫn bị bọc untrusted. | thấp | Theo dõi khi thêm domain mới cần chống SSRF |
| ~~K23~~ | ~~Cảnh báo mức cao M23 chưa nối vào `Router::notify`.~~ **ĐÃ XÓA (2026-09-26)** | Đã nối xong: `AlertSink` trait ở `bean-tools` (tool không phụ thuộc Router ⇒ không phụ thuộc vòng), `ToolCtx.alerts`, Router cài `RouterAlertSink` bọc quanh `notify`, cảnh báo `High` đi tới `alert_channel`/`alert_chat_id` và lỗi gửi rơi vào **outbox** như mọi outbound khác. Test: `security_scan_high_alert_reaches_the_main_channel` (gọi tool thật → cảnh báo thật tới channel), `high_severity_scan_sends_direct_alert`, `low_severity_scan_does_not_send_alert`, `alert_failure_does_not_break_the_tool`, `failed_alert_goes_to_outbox_instead_of_being_lost`. | — | — | — |
| ~~K24~~ | ~~**MCP server (M25) chưa có giới hạn tần suất và log truy cập chưa tách file riêng.** Đường `/mcp` xác thực bằng token dài hạn và audit **từng lời gọi tool**, nhưng chưa có `governor`/lockout như `POST /api/auth/login`, và log nằm chung `audit.jsonl` với `channel = mcp-client:<id>`.~~ **ĐÃ XÓA (2026-09-27)** | Đã vá xong. Chi tiết kỹ thuật: `docs/decisions.md` D16.10–D16.13. | **Đã vá:** (1) **tách thuật toán** giới hạn tần suất của `POST /api/auth/login` ra `bean_security::ratelimit::RateLimiter` và cho `AuthService` dùng lại — không viết thuật toán thứ hai, hành vi login không đổi (hàm đệ quy `check/record_failure/clear` còn nguyên, chỉ chuyển chỗ); (2) `McpRateLimiter` = 3 lớp: chống dò token (**đúng** ngưỡng login: 5 lần/60s, khoá tăng dần tới 300s, tính theo **cả** token lẫn IP) + trần lưu lượng 120 req/phút mỗi token + ×5 theo IP; (3) khoá theo `hash_token` **không** phải token thô; (4) `ServeContext` có hai cửa **bất đối xứng** — `authenticate_stdio` không có tham số limiter nào để truyền, `authenticate_http` thì kiểm tra giới hạn **trước** khi tra DB; (5) HTTP trả `429` + `Retry-After` (không phải `401`, để client tự thử lại thay vì báo "token sai"); (6) `audit/mcp.jsonl` riêng, ghi **song song** `audit.jsonl`. Cấu hình `[mcp_server].rate_limit_per_minute` / `rate_limit_ip_multiplier`; `http_enabled = true` mà đặt `0` thì validate chặn lúc nạp. Test: `core/tests/mcp_server.rs` (4 test K24) + `security/src/ratelimit.rs` (6 test) + `types/tests/config.rs` (2 test). Thiết kế: `D16.10`. | — | — | ~~trung bình~~ → **đã đóng** | Xong |

---

## 3. Môi trường kiểm thử — tiết kiệm thời gian cho người đọc sau

* **FTS5 đã có sẵn** trong `libsqlite3-sys` ở chế độ `bundled` (cờ `-DSQLITE_ENABLE_FTS5` trong
  `build.rs`). **Đừng** thêm `SQLITE3_CFLAGS`, `LIBSQLITE3_FLAGS` hay feature FTS5 giả — nếu
  migration báo "thiếu FTS5 trong SQLite?" thì đó là dấu hiệu bản build đã đổi, không phải thiếu cấu hình.
* `schemars` đã bật `derive` trong **default features** ⇒ không cần khai `features = ["derive"]` ở workspace.
* `/tmp` của môi trường dev đã từng bị dọn giữa hai lượt chạy ⇒ fixture kiểm thử E2E tạo lại mỗi lần, đừng cache.
* Kiểm chứng nhanh M5 bằng CLI thật:
  `printf '…\n' | ./target/debug/bean --config <toml tạm> chat --fake-llm <script>`;
  muốn xem **payload** gửi model thì trỏ `base_url` vào một mock HTTP server nhỏ (OpenAI-compat).
* Lint của repo: `unwrap`/`expect`/`panic` bị **deny** ngoài test, Rust 2024 nên `collapsible_if`
  muốn dùng let-chain (`if a && let Some(x) = …`) — `cargo clippy --workspace --all-targets -- -D warnings`
  phải sạch trước khi báo xong.

---

## 4. Việc cần nhặt lại theo milestone

* **S1 — ĐÃ XỬ LÝ (2026-09-26).** Vá xong: 5 tool đọc nội dung ngoài lõi đã bọc
  `<untrusted_content>` + bật cờ; thêm `Tool::marks_untrusted()` và test hồi quy
  (`docs/security-review.md` mục 2).
* **K1 — ĐÃ XỬ LÝ (2026-09-27).** Vá xong: `sessions.summary` bọc `<untrusted_content>`
  **và** bật `untrusted_seen` khi context có summary. Đây là phần còn lại của cùng lớp lỗi
  với S1 nhưng đi qua `context.rs` (`docs/decisions.md` D9.7).
* **S2 — ĐÃ XỬ LÝ (2026-09-27).** Vá xong: lọc escape terminal cho output của mọi tool
  `marks_untrusted()`, áp dụng ở CLI (`docs/security-review.md` mục 3.2, D9.8).
* **K24 — ĐÃ XỬ LÝ (2026-09-27).** Vá xong: giới hạn tần suất cho `POST /mcp` **chỉ** ở
  transport HTTP, tái dùng đúng thuật toán của `POST /api/auth/login` (rút ra
  `bean_security::ratelimit` cho cả hai dùng chung) + nhật ký riêng `audit/mcp.jsonl`
  ghi song song với `audit.jsonl` (`docs/decisions.md` D16.10–D16.13).
* **M6 (Skills)**: K9 (giới hạn cú pháp FTS trong description), K5 (nhiễu tool-call trong search).
* **M8 (Router/Channel)**: K2, K3, K6, K7, K13.
* **M9 (Web server)**: K6, K14 (REST cho phiên/memory files), giữ `/api/*` trả 404 JSON.
* **M15 (Learning loop — phần còn lại)**: K16 (tín hiệu phàn nàn), K17 (provider thật), K19 (audit/lịch sử draft), K20 (draft từ scheduler).
* **M16 (Hardening)**: K2, K3, K4, K11, K12, K18 + `make audit`/`make e2e` (K1 đã đóng 2026-09-27).
* **Sau tất cả milestone / trước phát hành**: K15 — smoke test Telegram thật bằng token lấy từ môi trường; không đưa token vào repo hay làm `make check` phụ thuộc secret; K21 — theo dõi future-incompatibility của `proc-macro-error2` qua `aquamarine`/`teloxide`.
