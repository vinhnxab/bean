# Bộ prompt cho coding agent — xây Bean (Rust + React UI)

Dùng được với Claude Code, Codex CLI, Cursor, Aider... Các prompt viết sẵn để **dán nguyên văn**, từng cái một. Đi kèm `agents.md` (v3).

---

## Chuẩn bị (làm một lần)

Cần cài sẵn: Rust (rustup, stable), Docker (cho sandbox), `cargo-audit` (hoặc `cargo-deny`). Chỉ để làm giao diện: Node LTS + `pnpm`. (Không có Node vẫn build được bản headless.)

```bash
mkdir Bean && cd Bean
git init
cp /đường/dẫn/agents.md ./agents.md        # Claude Code: đặt tên CLAUDE.md
claude                                          # hoặc codex / cursor / aider
```

**Mẹo vận hành**
- Mỗi milestone nên mở **một phiên mới** (hoặc `/clear`) để context gọn; agent sẽ đọc lại `agents.md`.
- Với Claude Code, bật plan mode (Shift+Tab) cho Prompt 0 và các milestone lớn (M4, M5, M8, M9, M10) để duyệt kế hoạch trước khi cho sửa file.
- Sau mỗi milestone tự chạy `make check` rồi mới sang milestone tiếp theo.
- **Kiểu API giữa Rust và React chỉ sửa ở Rust** (`bean-web`), rồi `make types`. Không sửa tay `web/src/api/generated/`.
- Nếu agent làm lan man: "Chỉ làm đúng milestone hiện tại theo agents.md mục 21."
- Nếu agent định thêm SSR/Next.js, server Node, Python, hoặc tải tài nguyên từ CDN: dừng lại, nhắc quy tắc ở mục 2 của agents.md.
- Với milestone UI (M10, M11) nên yêu cầu agent chạy `pnpm dev` cùng `bean serve --fake-llm` và tự kiểm tra bằng trình duyệt/test trước khi báo xong.

---

## Prompt 0 — Khởi động và lập kế hoạch (chưa viết code)

```
Đọc toàn bộ agents.md ở gốc repo. Đây là đặc tả một personal AI agent: backend Rust toàn bộ (một binary), giao diện React + TypeScript + Vite được nhúng vào binary khi release, runtime không có Node.

Chưa viết code. Hãy:
1. Tóm tắt kiến trúc bằng lời của bạn trong tối đa 15 dòng để tôi kiểm tra bạn hiểu đúng, nhất là: Router/Channel trong tiến trình, run thuộc về Router chứ không thuộc về kết nối WebSocket, và luồng xác nhận (confirm).
2. Liệt kê mọi điểm mơ hồ, mâu thuẫn hoặc thiếu sót trong spec (nếu có) kèm đề xuất xử lý.
3. Với mỗi crate Rust ở mục 3.1 và package web ở mục 3.2: kiểm tra còn được bảo trì, phiên bản hiện hành, tên feature cần bật (ví dụ rusqlite "bundled" có FTS5 không, ts-rs cách xuất kiểu, rust-embed chế độ debug/release, axum ws, teloxide, rmcp API hiện tại, cách cài Tailwind + shadcn/ui với Vite hiện nay). Đọc source/docs đã tải, không dựa vào trí nhớ. Đề xuất thay thế nếu có vấn đề.
4. Đề xuất kế hoạch chi tiết cho M1 và M2 (danh sách file sẽ tạo, test sẽ viết).

Dừng lại chờ tôi duyệt trước khi thay đổi bất cứ thứ gì.
```

---

## M1 — Skeleton Rust + web

```
Thực hiện milestone M1 theo agents.md (mục 3, 4, 5, 18, 21). Chỉ làm M1.

Việc cần làm:
- Cargo workspace với các crate ở mục 4 (tạo đủ crate, phần lớn để rỗng nhưng biên dịch được). `#![forbid(unsafe_code)]` cho mọi crate.
- bean-types theo mục 5; config.rs đọc bean.toml bằng toml + serde, validate, API key/token đọc từ biến môi trường theo trường *_env. Tạo bean.example.toml theo mục 18.
- bean-llm: trait LlmProvider + FakeProvider (trả lần lượt danh sách LlmResponse, nạp được từ file JSON).
- Binary `bean` (crate `bean`) với clap: `chat` là REPL đơn giản (rustyline) dùng FakeProvider, tạm chỉ echo; `serve` và `auth` để khung "chưa cài đặt".
- web/: khởi tạo bằng Vite + React + TypeScript strict, pnpm, Biome, Tailwind, Vitest + Testing Library. Một trang trống. vite.config.ts proxy /api (kể cả WebSocket) tới 127.0.0.1:7878. Font tự host, không CDN.
- Makefile với các target ở mục 3.3 (make types, check-rust, check-web, check, audit, build, build-headless; e2e là placeholder in "chưa có").
- Feature Cargo `ui` (mặc định bật) cho việc nhúng UI; build được với --no-default-features mà không cần thư mục web/dist.
- Test: config (thiếu env, sai kiểu), FakeProvider; một test Vitest tối thiểu cho web.

Định nghĩa xong: `make check` xanh; `cargo run -p bean -- chat` chạy; `pnpm dev` hiện trang; `cargo build --no-default-features` chạy khi không có Node. Commit "feat(m1): skeleton". Báo cáo theo mục 0.7.
```

---

## M2 — LLM providers

```
Thực hiện milestone M2 theo agents.md (mục 3.1, 5). Chỉ làm M2.

Việc cần làm:
- bean-llm: AnthropicProvider (Messages API) và OpenAiCompatProvider (Chat Completions, hỗ trợ base_url để dùng Ollama/OpenRouter), cùng implement LlmProvider. Tự viết trên reqwest + serde (rustls). Đọc tài liệu API chính thức để dùng đúng cấu trúc tool_use/tool_result và tool_calls; không đoán.
- Chuyển đổi hai chiều Message <-> định dạng của từng API, gồm cặp tool_use/tool_result và is_error.
- Retry với backoff + jitter, tối đa 3 lần cho 429/5xx/lỗi mạng, tôn trọng retry-after, không retry 4xx khác.
- API key dùng secrecy::SecretString; test rằng Debug/log không lộ key.
- Factory tạo provider từ config.
- Test bằng wiremock: chuyển đổi request/response đúng, retry đúng, lỗi 4xx không retry, phân tích stop_reason. Test gọi API thật đánh dấu #[ignore].
- Nối vào `bean chat`: chat một lượt bình thường với model thật (chưa có tool).

Định nghĩa xong: make check xanh; nếu có API key, `cargo test -- --ignored` chạy được. Commit "feat(m2): llm providers".
```

---

## M3 — Tool, registry, agent loop

```
Thực hiện milestone M3 theo agents.md (mục 6 và 7). Chỉ làm M3.

Việc cần làm:
- bean-tools: trait Tool, enum Risk, ToolCtx, TypedTool<P> với schemars (doc comment thành description, deny_unknown_fields), ToolRegistry.
- Tool file: read_file, list_dir, glob, grep (Safe); write_file, edit_file (Confirm). Có offset/limit. Tạm dùng đường dẫn tương đối với workspace và kiểm tra đơn giản; path jail bằng cap-std làm ở M4, nhưng đặt sau một trait/hàm để thay dễ.
- bean-core: agent loop đúng mục 6 (max_steps, lỗi tool thành tool result is_error, timeout, cắt output ở ranh giới UTF-8, chống lặp, CancellationToken kèm tool result giả khi huỷ). Trait RunIo (on_text, on_tool_start, on_tool_end, confirm).
- Trait Store cho lịch sử, tạm cài bản in-memory (SQLite làm ở M5).
- agent::prompt theo mẫu mục 19 (skills và memory để trống).
- Nối vào `bean chat`: hiển thị tiến trình tool; tool Confirm thì hỏi y/n trên terminal.
- Test bằng FakeProvider: kết thúc đúng; dừng ở max_steps; tool lỗi/không tồn tại/tham số sai/tham số thừa không làm crash và trả lỗi rõ ràng; cắt output không panic với tiếng Việt và emoji; chống lặp; huỷ giữa chừng vẫn hợp lệ.

Định nghĩa xong: make check xanh; demo "tạo hello.txt trong workspace rồi đọc lại" với model thật. Commit "feat(m3): tools and agent loop".
```

---

## M4 — Bảo mật lõi

```
Thực hiện milestone M4 theo agents.md (mục 7.2 và 15.1-15.4, 15.6, 15.8). Chỉ làm M4. Lập kế hoạch trước, chờ tôi duyệt, rồi mới code.

Việc cần làm:
- bean-security::paths: mọi thao tác file đi qua cap_std::fs::Dir gốc là workspace (nếu cap-std không phù hợp, nêu lý do và dùng canonicalize + kiểm tra prefix kèm xử lý symlink). Áp dụng cho MỌI tool file, thay phần tạm ở M3.
- bean-security::sandbox + tool run_shell (Confirm): docker run --rm với mount workspace, user non-root, --network none mặc định, --memory/--cpus/--pids-limit, --cap-drop ALL, no-new-privileges, container có tên để docker kill khi timeout, không truyền env host. Chế độ host phải bật tường minh và khi đó mọi lệnh là Dangerous. Trả stdout/stderr/exit code, cắt output dài.
- bean-security::policy: xử lý Safe/Confirm/Dangerous, "cho phép tool này trong phiên", deny-list mẫu (lớp phụ), audit log JSONL với redact secret.
- Cơ chế untrusted: hàm bọc <untrusted_content> (escape thẻ đóng trong nội dung) và cờ untrusted_seen trong lượt; sau khi cờ bật, tool Confirm trở lên luôn hỏi lại.

Test bắt buộc: path traversal và symlink escape (proptest), đường dẫn tuyệt đối, deny-list, sandbox không thấy file ngoài workspace, không có mạng khi network=false, timeout giết được container, audit log không lộ secret, "cho phép trong phiên" bị vô hiệu sau khi đọc untrusted.

Định nghĩa xong: toàn bộ test bảo mật pass, make check xanh. Commit "feat(m4): security layer".
```

---

## M5 — Bộ nhớ

```
Thực hiện milestone M5 theo agents.md (mục 8). Chỉ làm M5. Lập kế hoạch trước, chờ tôi duyệt.

Việc cần làm:
- bean-memory: SQLite bằng rusqlite (bundled), WAL, FTS5 + trigger đồng bộ, schema và migration theo user_version (mục 8.1, gồm cả các bảng outbox và web_sessions để sau dùng). Một thread/handle ghi duy nhất, không chặn runtime async. Cài trait Store đã tạo ở M3.
- Session theo (channel, chat_id) với archived/title như schema; ghi mỗi message ngay khi phát sinh.
- Context builder theo mục 8.2 (system + MEMORY.md/USER.md + summary + message gần nhất trong ngân sách token).
- Compaction theo mục 8.3: tóm tắt bằng LLM khi vượt 70% ngân sách; TUYỆT ĐỐI không cắt giữa cặp assistant(tool_calls) và tool results. Viết hàm tìm "ranh giới an toàn" và kiểm thử kỹ, gồm proptest sinh lịch sử ngẫu nhiên.
- Tool memory_save, memory_search (FTS5; làm sạch query để không lỗi cú pháp MATCH; tìm cả messages_fts).
- CLI: `/new` archive session cũ và tạo session mới.

Test bắt buộc: FTS5 tìm đúng và xếp hạng hợp lý (kèm tiếng Việt có dấu); khởi động lại vẫn giữ lịch sử; compaction không tách cặp tool; query có ký tự đặc biệt không làm lỗi.

Định nghĩa xong: tắt/bật lại CLI vẫn nhớ; hội thoại dài tự nén không lỗi API. Commit "feat(m5): memory".
```

---

## M6 — Skills

```
Thực hiện milestone M6 theo agents.md (mục 9). Chỉ làm M6.

Việc cần làm:
- bean-skills: loader quét ./skills và ~/.bean/skills, parse frontmatter (chọn crate YAML còn được bảo trì, KHÔNG dùng serde_yaml vì đã ngừng phát triển; hoặc tự parse tối giản hai trường name/description và nêu quyết định), validate (kebab-case, khớp tên thư mục, description ≤ 300 ký tự), bỏ qua và log skill lỗi.
- Đưa danh sách "name: description" vào system prompt, KHÔNG đưa nội dung đầy đủ.
- Tool load_skill (Safe) trả về nội dung SKILL.md kèm đường dẫn thư mục; create_skill (Confirm) chặn ghi đè và tên chứa ký tự đường dẫn.
- Viết 2 skill mẫu trong skills/: `web-research` và `daily-briefing`.
- Test loader (hợp lệ/không hợp lệ), load_skill, create_skill, và test bằng FakeProvider rằng model gọi load_skill rồi làm theo.

Định nghĩa xong: hỏi agent một việc khớp skill thì nó tự nạp skill. Commit "feat(m6): skills".
```

---

## M7 — Web tools

```
Thực hiện milestone M7 theo agents.md (mục 7.3 và 15.5). Chỉ làm M7.

Việc cần làm:
- bean-security::ssrf: chỉ http(s); resolver DNS tuỳ biến cho reqwest lọc IP ngay lúc kết nối (chống DNS rebinding); chặn private/loopback/link-local/metadata và IPv6 tương ứng; redirect policy tuỳ biến kiểm tra lại từng bước; giới hạn kích thước body và thời gian.
- Tool web_fetch (HTML -> text sạch, giới hạn độ dài, offset để đọc tiếp) và web_search (provider cắm được: tavily/brave/searxng theo config). Kết quả cả hai bọc <untrusted_content> và bật untrusted_seen.
- Test SSRF: 127.0.0.1, localhost, 169.254.169.254, 10.x/192.168.x, IPv6 loopback, domain trỏ về IP nội bộ, redirect vào IP nội bộ, scheme file://. Dùng wiremock/server cục bộ cho test, không gọi mạng thật.

Định nghĩa xong: test SSRF pass; agent tìm và tóm tắt được một trang web thật kèm nguồn. Commit "feat(m7): web tools".
```

---

## M8 — Router và Channel

```
Thực hiện milestone M8 theo agents.md (mục 10). Chỉ làm M8. Lập kế hoạch trước, chờ tôi duyệt.

Việc cần làm:
- bean-core::router: struct Router, Incoming, RunEvent, Decision, trait Channel đúng như mục 10. Ánh xạ (channel, chat_id) -> session (chọn session chưa archived mới nhất); hàng đợi/khoá mỗi session, run đến sau phát Queued.
- Run thuộc về Router: hàm submit trả RunId ngay, chạy nền; sự kiện phát qua tokio::sync::broadcast (xử lý RecvError::Lagged); confirm_id ngẫu nhiên gắn với run, phản hồi đầu tiên thắng, timeout mặc định 300 giây = DENY; cancel tường minh mới huỷ run.
- Slash command xử lý trong lõi không gọi LLM: /new /stop /model /skills /memory /tasks (/approve, /reject để khung, làm ở M15). Kiểm tra allowed_users.
- notify + bảng outbox: gửi lỗi thì lưu và thử lại có backoff.
- Refactor `bean chat` thành một Channel (channel "cli", chat_id "local") dùng Router, Ctrl-C = cancel.
- Test: hàng đợi theo phiên, run không bị huỷ khi subscriber rớt, confirm hết hạn thành DENY, phản hồi confirm đầu tiên thắng, cancel giữ cặp tool hợp lệ, /new archive session cũ, người dùng ngoài allowed_users bị từ chối, outbox thử lại.

Định nghĩa xong: make check xanh; CLI hoạt động như trước nhưng đi qua Router. Commit "feat(m8): router and channels".
```

---

## M9 — Web server (Rust)

```
Thực hiện milestone M9 theo agents.md (mục 11 và 15.7). Chỉ làm M9. Lập kế hoạch trước, chờ tôi duyệt. Đây là bề mặt tấn công nghiêm trọng vì agent có quyền chạy lệnh: ưu tiên đúng và an toàn hơn tính năng.

Việc cần làm:
- bean-web (axum): định nghĩa TẤT CẢ kiểu request/response/ClientMsg/ServerMsg trong Rust, derive ts-rs, xuất vào web/src/api/generated/ qua `make types`. Kiểm tra CI: git diff --exit-code trên thư mục đó.
- Xác thực: `bean auth set-password` (argon2id, lưu data.dir/auth.toml quyền 0600); `serve` từ chối bật web nếu chưa có mật khẩu. Login: so sánh thời gian không đổi, giới hạn tần suất theo IP + lockout tăng dần; token phiên 256-bit, CHỈ lưu hash trong web_sessions; cookie HttpOnly, SameSite=Strict, Secure khi public_origin là https; TTL; logout xoá phía server. Ghi audit sự kiện đăng nhập.
- Middleware: kiểm tra Origin/Host khớp public_origin cho request thay đổi dữ liệu, yêu cầu Content-Type application/json, không bật CORS, giới hạn kích thước body, header bảo mật ở mục 15.7 (CSP, nosniff, Referrer-Policy, no-store cho /api).
- REST theo bảng 11.1 (phần nào phụ thuộc milestone sau như skills/drafts thì để 501 có kiểu rõ ràng). /api/* không tồn tại trả 404 JSON.
- WebSocket /api/ws theo 11.2: kiểm tra Origin + cookie TRƯỚC khi nâng cấp, giới hạn kích thước message, ping/pong, Sync ngay sau khi kết nối, broadcast sự kiện Router tới mọi kết nối (xử lý Lagged bằng cách gửi lại Sync), ConfirmResolved, preview cắt đúng ranh giới UTF-8.
- Phục vụ UI: rust-embed sau feature `ui`, SPA fallback KHÔNG che /api, cache header (assets dài hạn, index.html no-cache). Nếu web/dist chưa có thì dùng trang placeholder để build không lỗi.
- `bean serve` khởi động web (Telegram/scheduler chưa cần), hỗ trợ --fake-llm.
- Test theo mục 20 (phần Web server): 401, sai Origin -> 403 (REST và WS), WS thiếu cookie bị từ chối, giới hạn đăng nhập, cờ cookie, header, /api 404 JSON, hash token phiên, Sync sau khi nối lại, sự kiện tới mọi kết nối, run không bị huỷ khi WS rớt.

Định nghĩa xong: toàn bộ test web-server pass; dùng client WS thử chạy trọn một lượt có tool cần xác nhận; make check xanh. Commit "feat(m9): web server".
```

---

## M10 — UI: đăng nhập và chat

```
Thực hiện milestone M10 theo agents.md (mục 12.1 màn 1-4, 12.2, 12.3). Chỉ làm M10. Lập kế hoạch trước, chờ tôi duyệt (cấu trúc thư mục web/src, danh sách component, thư viện cụ thể).

Việc cần làm:
- Trang đăng nhập; router có route bảo vệ; api/client.ts (fetch same-origin, 401 -> về đăng nhập); api/ws.ts (tự nối lại với backoff, ping, xử lý Sync, nạp lại lịch sử bằng REST sau khi nối lại, khử trùng theo message_id/run_id).
- store/chat.ts (zustand) đúng mục 12.2; lịch sử lấy từ REST làm nguồn sự thật.
- Màn Chat: thanh bên danh sách hội thoại (mới, chọn; tìm/đổi tên/lưu trữ/xoá để M11), danh sách tin nhắn có phân trang ngược, ô nhập (Enter gửi, Shift+Enter xuống dòng), nút Dừng, trạng thái Queued.
- Thẻ tool (mục 12.1.3) và Thẻ xác nhận (mục 12.1.4) đúng như mô tả: hiển thị nguyên văn hành động bằng monospace, nhãn rủi ro, ẩn "trong phiên" khi không cho phép, đếm ngược, khoá sau khi giải quyết, không phím tắt cho phép, không tự focus nút Cho phép.
- Markdown renderer an toàn theo mục 12.3: react-markdown không HTML thô, ảnh từ xa hiển thị thành liên kết văn bản kèm cảnh báo, chỉ cho phép http/https/mailto, rel noopener noreferrer, output tool hiển thị dạng text trong <pre>. Không dùng dangerouslySetInnerHTML.
- Giao diện tối/sáng theo hệ thống, responsive (kiểm tra ở chiều rộng điện thoại), aria/bàn phím cơ bản, i18n vi (mặc định) và en.
- Test Vitest theo mục 20 (phần Web): markdown độc hại (script, img onerror, javascript:, ảnh từ xa), thẻ xác nhận, WS nối lại + Sync + không nhân đôi tin, chuỗi sự kiện Queued -> ToolStart -> ToolEnd -> Final, 401 -> đăng nhập, đủ khoá i18n.
- Tự kiểm tra thực tế: chạy `bean serve --fake-llm <kịch bản có tool cần xác nhận>` cùng `pnpm dev`, thao tác trọn luồng và mô tả kết quả.

Định nghĩa xong: chat và duyệt/từ chối hành động được trên trình duyệt (cả desktop lẫn màn hình hẹp); make check xanh. Commit "feat(m10): web ui chat".
```

---

## M11 — UI: quản lý

```
Thực hiện milestone M11 theo agents.md (mục 12.1 màn 5-9). Chỉ làm M11.

Việc cần làm (dùng TanStack Query cho REST; kiểu lấy từ web/src/api/generated):
- Thanh bên chat: tìm hội thoại (FTS), đổi tên, lưu trữ, xoá (có xác nhận).
- Bộ nhớ: sửa MEMORY.md/USER.md (xác nhận trước khi lưu, cảnh báo thay đổi chưa lưu), danh sách ghi nhớ có tìm và xoá.
- Skills: danh sách và xem nội dung (phần nháp/duyệt để M15, hiển thị trạng thái "chưa có").
- Tác vụ định kỳ: danh sách, tạo, bật/tắt, xoá; hiển thị lần chạy tiếp theo theo múi giờ người dùng (phần chạy thật ở M13; nếu API chưa đủ thì bổ sung phía Rust và chạy lại make types).
- Audit: bảng chỉ đọc, phân trang. Trạng thái: phiên bản, model, ngân sách token, kênh đang chạy.
- Điều hướng, trạng thái loading/lỗi/rỗng nhất quán; responsive; i18n vi/en; hiển thị thời gian theo múi giờ trình duyệt.
- Test Vitest + MSW cho từng màn (tải, lỗi, thao tác chính).

Định nghĩa xong: đủ 9 màn hình ở mục 12.1 (trừ phần nháp skill), dùng được trên điện thoại; make check xanh. Commit "feat(m11): web ui management".
```

---

## M12 — Telegram

```
Thực hiện milestone M12 theo agents.md (mục 13). Chỉ làm M12. Lập kế hoạch trước, chờ tôi duyệt. Giữ adapter MỎNG: không logic agent, mọi thứ đi qua Router.

Việc cần làm:
- bean-channels::telegram bằng teloxide (long polling), implement Channel. Allowlist BẮT BUỘC (người lạ bị bỏ qua + log), chống trùng update, giới hạn tần suất mỗi chat, tách tin > 4096 ký tự ở ranh giới an toàn (không cắt giữa ký tự UTF-8), typing định kỳ, xác nhận bằng inline keyboard (Cho phép / Trong phiên / Từ chối) với callback_data chỉ chứa confirm_id ngắn, chỉ chấp nhận callback từ user_id đã cấp phép, hết hạn thì sửa tin thành "Hết hạn". Mặc định gửi văn bản thuần (nếu dùng MarkdownV2 phải escape đúng).
- /stop huỷ run đang chạy; tự nối lại khi rớt; phát hiện lỗi 409 (hai instance) và log rõ; tắt êm theo CancellationToken.
- `bean serve` chạy web và Telegram cùng lúc (theo config).
- Test bằng mock (trait trừu tượng lớp Bot hoặc server giả): allowlist, rate limit, tách tin, confirm hết hạn, callback từ người lạ bị từ chối. Không cần token thật để chạy test.

Định nghĩa xong: chat và duyệt hành động qua Telegram thật được; cùng một agent dùng được đồng thời trên web và Telegram. Commit "feat(m12): telegram".
```

---

## M13 — Scheduler và tin chủ động

```
Thực hiện milestone M13 theo agents.md (mục 14 và 10). Chỉ làm M13.

Việc cần làm:
- bean-core::scheduler: task tokio tick 30 giây, trait Clock để test, croner/cron + chrono-tz, lưu UTC, parse cron theo agent.timezone. Task lỡ hạn thì bỏ qua và tính lần kế tiếp.
- Tool schedule_task(cron, prompt, allowed_tools), list_tasks, cancel_task (tạo/huỷ là Confirm). Hoàn thiện REST /api/tasks.
- Khi đến hạn: chạy agent trong session của task; run không có người xác nhận nên tool Confirm/Dangerous bị từ chối tự động trừ khi nằm trong allowed_tools đã duyệt. Kết quả gửi qua Router::notify: Telegram nhận tin; web nhận ServerMsg::Notification và hiển thị tin trong phiên tương ứng.
- Test với đồng hồ giả: đến hạn chạy đúng một lần, đúng múi giờ Asia/Ho_Chi_Minh, task bị huỷ không chạy, tool Confirm bị từ chối khi chạy nền, outbox giữ tin khi kênh lỗi rồi gửi bù.

Định nghĩa xong: tạo được "mỗi sáng 7h tóm tắt việc cần làm", nhận kết quả qua Telegram và thấy trong UI. Commit "feat(m13): scheduler".
```

---

## M14 — MCP client

```
Thực hiện milestone M14 theo agents.md (mục 16). Chỉ làm M14.

Việc cần làm:
- bean-tools::mcp dùng rmcp (SDK Rust chính thức; xác nhận API hiện tại từ source đã tải). Kết nối server stdio từ config, lấy danh sách tool, đăng ký với tên mcp__<server>__<tool>, chuyển JSON Schema thành ToolSpec.
- Rủi ro mặc định Confirm; chỉ Safe nếu trust = true. Kết quả bọc <untrusted_content> và bật untrusted_seen.
- Server lỗi/treo: timeout khi kết nối và khi gọi tool, log cảnh báo, bỏ qua, agent vẫn khởi động. Đóng tiến trình con sạch khi thoát.
- Hỗ trợ `command` là lệnh bọc container (docker run ...), ghi ví dụ vào README.
- Test bằng một MCP server giả tối giản viết bằng Rust ngay trong workspace (stdio); KHÔNG dùng server Node cho test.

Định nghĩa xong: kết nối được 1 MCP server thật (ưu tiên server không phải Node) và agent dùng được tool của nó. Commit "feat(m14): mcp client".
```

---

## M15 — Learning loop

```
Thực hiện milestone M15 theo agents.md (mục 17 và 12.1 màn 6). Chỉ làm M15.

Việc cần làm:
- Sau run thành công có ≥ 5 tool call (và chưa vượt giới hạn tần suất, mặc định 1 đề xuất/giờ, bật/tắt trong config): chạy lượt reflection bằng LLM, đánh giá quy trình có tái sử dụng được không; nếu có thì sinh SKILL.md hợp lệ theo mục 9.
- Lưu vào skills/_drafts/<name>/, thông báo qua Router::notify. Chỉ khi người dùng duyệt mới chuyển vào skills/ và nạp lại loader; không bao giờ tự kích hoạt. Hỗ trợ /approve <id>, /reject <id>, nút inline trên Telegram, và REST /api/skills/drafts.
- UI: màn Skills hiển thị skill nháp với diff/nội dung, nút Duyệt/Bỏ, và ô thông báo khi có đề xuất mới (Notification).
- Khi run có load_skill nhưng agent làm khác hướng dẫn và thành công: đề xuất bản sửa dạng diff, chờ duyệt.
- Test bằng FakeProvider: đủ điều kiện thì sinh nháp, không đủ thì không; nháp sai format bị loại; giới hạn tần suất hoạt động; không tự kích hoạt. Test UI cho màn duyệt nháp.

Định nghĩa xong: sau một task nhiều bước, nhận được đề xuất skill (UI và Telegram) và duyệt được. Commit "feat(m15): learning loop".
```

---

## M16 — Hardening và triển khai

```
Thực hiện milestone M16 theo agents.md (mục 15, 20, 23). Chỉ làm M16.

Việc cần làm:
- Ngân sách token/ngày và giới hạn bước: dừng và báo người dùng khi vượt (hiển thị trong UI và Telegram).
- Logging có cấu trúc (tracing), redact secret; tắt êm khi SIGTERM (Router, scheduler, kênh, web); tự nối lại Telegram/MCP khi rớt.
- Release: `make build` (build web -> cargo build --release với feature ui, lto = "thin", strip = true), binary tự chứa giao diện, kiểm tra chạy được trên máy KHÔNG có Node. `make build-headless` chạy được không cần Node.
- deploy/systemd/bean.service (user riêng, NoNewPrivileges, ProtectSystem, PrivateTmp...). Tài liệu hoá rằng thành viên nhóm docker gần như tương đương root và nêu phương án rootless Docker/Podman.
- Dockerfile nhiều tầng (tầng Node/pnpm build web, tầng Rust build nhúng UI, tầng cuối debian slim chỉ chứa binary, KHÔNG có Node) và Dockerfile cho image sandbox (Debian slim + công cụ cơ bản, KHÔNG Node). Nêu rõ rủi ro mount docker socket nếu chạy agent trong container.
- `make audit` (cargo audit/deny + pnpm audit --prod) chạy sạch; commit Cargo.lock và pnpm-lock.yaml.
- README: cài đặt, cấu hình, đặt mật khẩu web, truy cập từ xa an toàn (reverse proxy TLS hoặc Tailscale, public_origin), tạo bot Telegram (BotFather), thêm skill, thêm MCP server, mô hình đe doạ và giới hạn bảo mật.
- Chạy make check, make audit, make e2e (điền target e2e theo mục 20: đăng nhập -> chat -> confirm -> Stop -> đóng/nối lại WS nhận Sync -> người lạ bị chặn). Tuỳ chọn: một luồng Playwright trên trình duyệt thật.
- Smoke test chạy 1 giờ với scheduler tick nhanh.

Định nghĩa xong: cài bằng systemd chạy được, dùng được qua web và Telegram; make check/audit/e2e xanh. Commit "feat(m16): hardening and deploy".
```

---

## M17 — Streaming và kênh bổ sung (tuỳ chọn)

```
Thực hiện milestone M17 theo agents.md. Chọn MỘT phần mỗi lần.

(A) Streaming token:
- Thêm LlmProvider::chat_stream (SSE từ Anthropic/OpenAI-compat) trả về Stream các LlmDelta; giữ nguyên `chat` không stream.
- Agent loop gom delta thành LlmResponse; phát sự kiện Text delta qua Router -> WS (thêm biến thể ServerMsg::TextDelta, chạy make types).
- UI hiển thị chữ hiện dần, xử lý nối lại giữa chừng (nạp lại từ REST khi Final). Telegram vẫn chờ tin đầy đủ.
- Test bằng wiremock trả SSE; test UI cho ghép delta và ngắt kết nối giữa stream.

(B) Kênh Discord hoặc Slack:
- Implement trait Channel trong bean-channels bằng thư viện đã chốt (serenity/twilight hoặc slack-morphism); cùng chuẩn với Telegram: allowlist bắt buộc, rate limit, tách tin theo giới hạn nền tảng, xác nhận bằng cơ chế tương ứng, reconnect, tắt êm.
- Không sửa Router và không sửa API web trừ khi tôi đồng ý.
```

---

## Prompt dùng khi cần

### Tiếp tục ở phiên mới

```
Đọc agents.md và `git log --oneline`. Xác định các milestone đã hoàn thành. Chạy `make check` và cho tôi biết trạng thái hiện tại. Sau đó tóm tắt việc còn lại của milestone kế tiếp, chưa code.
```

### Review bảo mật (nên chạy sau M4, M7, M9, M10, M16)

```
Đóng vai người review bảo mật khó tính. Đọc code trong crates/bean-security, bean-tools, bean-core, bean-web, bean-channels và web/src. Tìm cách một attacker có thể:
(1) thoát khỏi workspace; (2) chạy lệnh mà không được xác nhận; (3) truy cập mạng nội bộ; (4) lộ secret;
(5) khiến agent làm theo chỉ dẫn trong nội dung untrusted (web, file, MCP);
(6) từ một trang web khác điều khiển agent qua trình duyệt của người dùng (CSRF, cross-site WebSocket hijacking, thiếu kiểm tra Origin);
(7) đánh cắp dữ liệu qua giao diện (XSS, ảnh markdown từ xa, liên kết độc hại);
(8) vượt xác thực hoặc chiếm phiên (brute force, token phiên lưu rõ, cookie thiếu cờ);
(9) giả mạo user_id/callback trên Telegram để duyệt hành động thay người dùng.
Với mỗi lỗ hổng: mô tả cách khai thác, mức độ, và viết TEST tái hiện (phải fail trước khi sửa) rồi mới sửa. Không sửa gì mà chưa có test.
```

### Review chất lượng theo từng phần

```
Review toàn bộ code theo agents.md mục 20 và 22.
Rust: unwrap/expect/panic ngoài test, khoá std::sync giữ qua .await, blocking trong async, cắt String theo byte, select! không cancel-safe, broadcast Lagged không được xử lý, task con không nhận CancellationToken, lịch sử có thể bị cắt tách cặp tool.
Web: `any`, dangerouslySetInnerHTML, tài nguyên tải từ ngoài, kiểu TS viết tay thay vì kiểu sinh, tin nhắn bị nhân đôi khi nối lại WS, thiếu trạng thái loading/lỗi, thiếu i18n, không dùng được trên màn hình hẹp.
Chung: output không giới hạn, test còn thiếu cho hành vi quan trọng.
Xếp theo mức ưu tiên, đề xuất sửa; chỉ sửa sau khi tôi đồng ý.
```

### Kiểm tra tuân thủ ràng buộc công nghệ

```
Rà soát repo theo agents.md mục 2: (1) Node/npm/pnpm có xuất hiện ngoài thư mục web/ và ngoài bước build/test UI không? Binary release có phụ thuộc Node lúc chạy không? (2) có SSR/Next.js/Nuxt hoặc server Node nào không? (3) có mã Python nào trong project không? (4) `cargo build --no-default-features` có build được mà không cần Node không? (5) có logic agent/bộ nhớ/quyết định quyền nào lọt vào adapter kênh (Telegram) hoặc vào web/ (TypeScript) thay vì ở lõi Rust không? (6) UI có tải tài nguyên từ CDN/domain ngoài (font, script, ảnh) không? Liệt kê vi phạm và đề xuất cách chuyển về đúng chỗ.
```

### Mẫu báo lỗi để agent debug

```
Lỗi: <mô tả ngắn>
Thành phần: <lõi | web server | UI | telegram | cli>
Cách tái hiện: <lệnh hoặc các bước>
Kết quả thực tế: <log/traceback/ảnh chụp console trình duyệt>
Kết quả mong đợi: <...>

Hãy: (1) viết một test tái hiện lỗi và xác nhận nó fail, (2) tìm nguyên nhân gốc (không vá triệu chứng), (3) sửa, (4) chạy lại `make check` (và `make e2e` nếu lỗi liên quan giao tiếp giữa UI và server). Nêu rõ nguyên nhân gốc trong báo cáo.
```

### Khi agent đi lệch hướng

```
Dừng. Bạn đang làm ngoài phạm vi milestone hiện tại (hoặc vi phạm ràng buộc công nghệ). Hoàn tác các thay đổi ngoài phạm vi (dùng git diff để kiểm tra), quay lại đúng danh sách việc của milestone và agents.md mục 0 và 2.
```

---

## (Tuỳ chọn) Lệnh slash tuỳ chỉnh cho Claude Code

`.claude/commands/milestone.md` — gõ `/milestone M3`:

```markdown
Thực hiện milestone $ARGUMENTS theo agents.md (mục 21 và các mục liên quan).

Quy trình:
1. Đọc agents.md, xem `git log` để biết trạng thái hiện tại.
2. Nêu kế hoạch ngắn (file sẽ tạo/sửa, test sẽ viết), chờ tôi xác nhận nếu milestone thuộc M4, M5, M8, M9, M10, M12.
3. Viết test và code. Chỉ làm đúng milestone này, tuân thủ ràng buộc công nghệ ở mục 2 (backend Rust; UI React trong web/; không SSR, không Python; runtime không cần Node).
4. Chạy `make check` (và `make e2e` nếu milestone liên quan đến web/WebSocket). Với milestone UI: chạy thử trên trình duyệt và mô tả kết quả. Sửa đến khi xanh hết.
5. Commit theo quy ước và báo cáo: đã làm gì, cách kiểm tra, việc tồn đọng.
```

`.claude/commands/security-review.md`:

```markdown
Đóng vai người review bảo mật khó tính cho thay đổi hiện tại (`git diff main`). Tìm lỗ hổng theo agents.md mục 15 (gồm cả bề mặt web: xác thực, CSRF, WebSocket Origin, XSS/ảnh từ xa qua Markdown). Với mỗi lỗ hổng, viết test tái hiện trước rồi mới đề xuất sửa.
```
