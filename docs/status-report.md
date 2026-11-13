# Báo cáo trạng thái Bean

> **Ghi chú đổi tên (2026-09-28).** Đổi tên `BeanAgent`/`beanagent-*` → `bean`/`bean-*` trên
> toàn repo (xem `docs/decisions.md`, mục 19 — D18.x). Báo cáo này mô tả trạng thái tại
> commit `62e8350`; sau rename, **lệnh và tên crate trong báo cáo đã được cập nhật sang tên
> mới** để còn chạy được. Tag `pre-rename-beanagent` giữ nguyên trạng thái commit gốc mà
> báo cáo này mô tả, dùng khi cần tái hiện đúng môi trường cũ. Các **mã commit** trong
> báo cáo (`62e8350`, `882363d`…) là dữ liệu git thật nên **không** đổi.

**Ngày cập nhật:** 2026-09-25
**Nhánh:** `master`
**HEAD khi lập báo cáo:** `62e8350` (`docs: track rust future-incompatibility warning`)
**Phạm vi:** milestone M1–M17 và mức độ sẵn sàng vận hành của phiên bản hiện tại.

> **Cập nhật bổ sung 2026-11-02** (chỉ tài liệu, **không re-run `make check`** — bảng mục 4 giữ
> trạng thái của báo cáo gốc): repo đã đi tiếp **M21–M28** — RBAC theo tag (M21), monitor /
> finance / security-scan / marketing (M22–M24), Bean làm MCP server (M25), tool browser CDP
> (M26), QA test-runner **và** voice push-to-talk (M27 — hai tính năng cùng mang nhãn này trong
> code/tài liệu), đổi tên `BeanAgent` → `bean` (M28). Vận hành thật đã xác minh: chat qua
> OpenRouter (`openai_compat`) chạy ổn sau khi đổi model, ElevenLabs STT/TTS hoạt động. Bảng
> mục 2 dưới đây dừng ở M25; M26–M28 xem các dòng bổ sung cuối bảng.

> Báo cáo phân biệt rõ **đã có mã nguồn + test tự động** với **đã kiểm chứng trên hệ thống thật**. Nhiều chức năng đã triển khai và xanh test, nhưng chưa có credential để chạy với LLM thật, Telegram thật hoặc thời gian vận hành dài.

## 1. Tóm tắt điều hành

- **M1–M16:** phần lớn chức năng theo đặc tả v1 đã được viết, tích hợp và kiểm thử tự động.
- **M17(A) — streaming token:** đã hoàn thành cho Anthropic và OpenAI-compatible, gồm SSE parser, `TextDelta` qua Router/WebSocket, UI hiện chữ dần và test reconnect/refetch.
- **Kênh chat:** Telegram + web là bộ cuối cùng. **M18 (Discord) đã bị loại bỏ ngày 2026-09-26** — không có trong roadmap (`Plan.md` mục 5.0).
- **Cổng chất hiện tại:** `make check`, `make audit` và `make e2e` đều xanh; release build và headless build thành công.
- **Điểm yếu mức cao theo `docs/known-issues.md`: đã đóng hết.** Trước đây điểm yếu lớn nhất còn lại là K1 (mức cao) — nay **đã khắc phục 2026-09-27**. Việc còn mở nằm ở nhóm độ bền DB và kiểm thử thực tế, không phải bảo mật.
- **S1 — prompt injection qua `read_file`/`grep`/`glob`/`list_dir`/`run_shell` — đã khắc phục (2026-09-26).** Chi tiết: `docs/security-review.md` mục 2, `docs/decisions.md` D9.1.
- **K1 — prompt injection qua `sessions.summary` — đã khắc phục (2026-09-27).** Đây là nửa còn lại của cùng lớp lỗi với S1 nhưng đi qua `context.rs`. Vá: bọc `<untrusted_content>` **và** bật `untrusted_seen` khi context có summary. Đánh đổi đã được duyệt: phiên đã compact mất "cho phép trong phiên" cho tool `Confirm`/`Dangerous` tới hết phiên (`docs/decisions.md` D9.7, `docs/known-issues.md` K1 + K1-followup).
- **S2 — terminal escape injection qua CLI — đã khắc phục (2026-09-27).** Lọc trọn chuỗi ESC/OSC cho output của mọi tool `marks_untrusted()` trước khi in ở CLI; tool không untrusted giữ nguyên hành vi (`docs/decisions.md` D9.8, `docs/security-review.md` mục 3.2).
- **Còn mở trong báo cáo review:** S3 (`/api/audit` chưa lọc theo người dùng) và S4 (`allowed_tools` cấp quyền vĩnh viễn) — đều là **ghi chú thông tin**, cần rà lại khi chuyển multi-user / khi thêm cảnh báo UI.
- Chưa nên coi là production-ready hoàn toàn vì chưa có kiểm thử provider thật, Telegram thật, soak test 24 giờ, E2E memory/learning đầy đủ, CI và triển khai thực tế.

## 2. Trạng thái theo milestone

| Milestone | Trạng thái | Commit | Đã hoàn thành | Phần còn thiếu hoặc chưa chứng minh |
|---|---|---|---|---|
| **M1 — Skeleton** | ✅ Hoàn thành | `d5ef85a` | Cargo workspace 10 crate, config TOML, types, `FakeProvider`, CLI `chat/serve/auth`, web Vite/React/TS, Makefile | — |
| **M2 — Provider** | 🟡 Code + test xanh | `53a8d3c` | Anthropic Messages API, OpenAI-compatible, retry/backoff, wiremock; M17 bổ sung SSE | Chưa có một lượt chat với API key thật (K12) |
| **M3 — Tools + agent loop** | ✅ Hoàn thành | `81632aa` | Registry, file/shell tools, giới hạn bước, timeout, cancel, chống lặp, audit flow | — |
| **M4 — Security** | ✅ Tự động | `5545cc2` | Path jail, Docker sandbox, policy Safe/Confirm/Dangerous, allow-in-session, deny-list, audit, untrusted wrapper | K1 đã đóng 2026-09-27 (D9.7); rủi ro môi trường thật cần vận hành theo README |
| **M5 — Memory** | 🟡 Code + test xanh | `c22bde5`, `b29ae73` | SQLite, FTS5, context budget, compaction an toàn, `MEMORY.md`/`USER.md`, memory tools, `/new`, persistence | Chưa có E2E memory bền vững (K11) và hội thoại dài với provider thật (K12) |
| **M6 — Skills** | 🟡 Code + fake flow | `0a1187c` | Loader, progressive disclosure, `load_skill`, `create_skill`, skill draft | Chưa chứng minh model thật luôn nạp và làm theo skill |
| **M7 — Web tools** | ✅ Tự động | `2582f6c` | `web_fetch`, `web_search`, HTML→text, untrusted wrapper, chống SSRF/redirect/DNS rebinding | Chưa smoke test với API tìm kiếm thật |
| **M8 — Router/Channel** | ✅ Hoàn thành | `d85aac4` | Router, queue theo session, cancel, confirm, outbox retry, slash commands ở lõi, CLI qua Router | K13 trong known-issues đã cũ; `/new` thực tế đã ở Router |
| **M9 — Web server** | ✅ Hoàn thành | `41e105d` | Auth Argon2id + cookie, REST, WebSocket, `ts-rs`, CSP/Origin/CSRF/rate limit, SPA embedded | K14 đã cũ; API/UI memory file đã tồn tại |
| **M10 — UI chat** | 🟡 Test xanh | `7ad3c42` | Login, chat, tool card, confirm card, Stop, realtime state | Chưa có Playwright/browser test thật |
| **M11 — UI quản lý** | 🟡 Test xanh | `d0ab3db` | Sessions, memory, skills, tasks, audit, status; responsive; i18n `vi`/`en` | Chưa kiểm chứng responsive bằng trình duyệt thật |
| **M12 — Telegram** | 🟡 Adapter + mock test | `810577f` | teloxide long polling, allowlist, rate limit, tách tin UTF-16, inline confirm, reconnect, shutdown | **Chưa chạy bot Telegram thật** (K15) |
| **M13 — Scheduler** | 🟡 Logic + smoke ảo | `cbcd744` | Cron/timezone, task CRUD, scheduler tick, notification, outbox | Chưa chứng minh “7h mỗi sáng” qua Telegram/UI thật |
| **M14 — MCP** | ✅ Local stdio server | `7ef375a` | rmcp stdio client, discovery, trust/confirm, reconnect, reaping, MCP test server thật trong tiến trình | Chưa thử MCP server bên thứ ba trong môi trường triển khai |
| **M15 — Learning loop** | 🟡 Gần hoàn tất | `a9052ba` | Reflection, draft, rate limit, duyệt/bỏ qua UI, REST và Telegram; reflection trì hoãn đến lượt kế tiếp (K16, D28) | K20 (scheduler draft chưa có inline Telegram); luồng 2 lượt K16 chưa chạy lại với provider thật |
| **M16 — Hardening/deploy** | 🟡 Gần hoàn tất | `8e83849` | Token budget, structured logs, graceful shutdown, reconnect, embedded UI, headless build, systemd, Dockerfiles, README, audit, binary E2E | **Chưa soak 24 giờ**; chưa cài systemd production; chưa đẩy image production; chưa chạy Playwright |
| **M17(A) — Streaming** | ✅ Hoàn thành | `882363d` | SSE Anthropic/OpenAI-compat, gom delta/tool call/usage, `TextDelta` qua WS, UI hiện dần, reconnect giữ text và refetch REST | Chưa kiểm thử thủ công với provider thật do chưa có API key |
| **M18 — Discord adapter** | 🚫 Đã loại bỏ | — | — | Chủ dự án bỏ hẳn 2026-09-26: Discord không có tín hiệu API chặn trùng token (Telegram trả 409) và có vòng đời interaction riêng (3s/15 phút). Xem `Plan.md` mục 5.0 |
| **M21 — RBAC + project profile** | ✅ Hoàn thành | `6a5d063` | `RolePermissions` tuần tự hoá được, resolve **một lần** ở Router, `required_tags` lọc tool trước khi gọi LLM, `no-access` = deny-all, `[[projects]]` tách workspace/MEMORY.md, `usage_by_role` | `GET /api/status` chưa hiện hạn mức per-role |
| **M22 — Monitor agent** | 🟡 Code + test xanh | `35bb76c` | Tag RBAC gắn được ở cấp `[[mcp_servers]].tool_tags`; MCP test server có tool chỉ đọc (SIEM, CVE); dùng lại cơ chế MCP stdio sẵn có, **không** thêm tool quét chủ động | Chưa nối MCP server SIEM/CVE thật vào môi trường triển khai |
| **M22a — Finance-readonly** | 🟡 Code + test xanh | `3982885` | Tool `billing_read_cost` chỉ đọc, hướng **generic** (endpoint + credential từ cấu hình), chế độ stub khi chưa cấu hình, `validate()` chặn dùng chung biến credential với LLM/search/Telegram | Chưa điền endpoint/credential thật; cần bạn xác nhận nhà cung cấp cloud |
| **M23 — Security-scan** | 🟡 Code + test xanh, chưa bật | (commit này) | `[[infra_scope]]` kiểm ở **tầng code** trước khi spawn scanner, rỗng = từ chối mọi thứ; chỉ nhận IP/CIDR (chống TOCTOU); code tự dựng argv + `Sandbox::run_argv` (không `sh -c`); sandbox riêng có mạng; `Dangerous` ⇒ không cho phép trong phiên; output bọc untrusted | `[[infra_scope]]` **đang rỗng** (bạn chọn fail-closed) nên chưa quét được gì thật; cảnh báo mức cao chưa nối `Router::notify` (K23) |
| **M24 — Marketing** | ✅ Hoàn thành | (commit này) | Cơ chế `allowed_tool_tags` (danh sách trắng theo role) + `Tool::also_visible_to`; `marketing_draft` (Confirm, **không gọi mạng**), `marketing_publish` (Dangerous cứng trong code), role `marketing` tách domain; hướng dẫn content-integrity chèn vào system prompt (M24 mục 5) | Chưa điền endpoint/credential thật |
| **M25 — Bean làm MCP server** | 🟡 Code + test xanh, đã smoke test thật | (commit này) | `mcp serve` (stdio) và `mcp serve --http` (streamable-HTTP/SSE); token dài hạn lưu **hash** trong bảng `mcp_clients` (migration v6), lệnh `auth mcp-token add/list/revoke`; identity `mcp-client:<id>`; cổng expose cứng 3 tag + `Risk::Safe` **chặn trước** RBAC nên role `admin` cũng không gọi được tool ghi; tool mới `memory_query`; `Router::call_tool_as` tái dùng `RolePermissions::allows`; tham số client được làm sạch ở ranh giới duy nhất | ~~`/mcp` chưa rate-limit, log truy cập chưa tách file riêng (K24)~~ — **K24 đã đóng 2026-09-27**: rate-limit theo token + IP **chỉ** cho transport HTTP (tái dùng thuật toán khoá của `POST /api/auth/login`), `audit/mcp.jsonl` ghi song song `audit.jsonl` (`D16.10`–`D16.13`); chưa thử với Cline/Cursor **thật** (mới chỉ smoke test bằng JSON-RPC thô) |
| **M26 — Tool browser nội bộ (CDP)** | 🟡 Code + test xanh | — | Nói thẳng Chrome DevTools Protocol, không tải Chrome lúc runtime; origin ngoài allowlist luôn `Dangerous`; output bọc untrusted | Chưa vận hành dài hạn trên nhiều site thật |
| **M27 — QA test-runner** | 🟡 Code + test xanh | — | `qa_test` chạy test suite có sẵn, workspace mount read-only; role `qa` không có đường ghi nào (four-eyes) | Chưa kiểm chứng với repo thật có suite lớn |
| **M27 — Voice push-to-talk** | 🟡 Đã chạy thật | — | `bean-speech`: STT/TTS `local` (whisper.cpp + Piper) hoặc `api` (ElevenLabs `scribe_v2` / `eleven_multilingual_v2`); lượt voice chỉ chạy tool `Risk::Safe`; key nhập qua UI lưu `data.dir/voice-state.json` (0600) | STT thật OK 2026-11-02 (trước đó `502` do kế thừa proxy env — K29); TTS premade đã test 200 ngày 2026-10-28 |
| **M28 — Đổi tên `BeanAgent` → `bean`** | ✅ Hoàn thành | — | Đổi tên crate/lệnh/đường dẫn toàn repo; không migration tự động: `mv ~/.BeanAgent ~/.bean`, `BEANAGENT_*` → `BEAN_*` (D18.x) | Tag `pre-rename-beanagent` giữ trạng thái gốc |
| **M29 — Thiết lập & secrets thống nhất** | ✅ Hoàn thành | — | Kho secret duy nhất `data.dir/secrets.env` (0600), thứ tự env > secrets.env; `bean secret/config/migrate-secrets`; bảng thiết lập tầng A/B + step-up mật khẩu; `bean doctor` kiểm quyền file, slot bị che, proxy (K29), tiến trình/port (K27); LAN access + host mode toggle (D21/D22) | ~~`~/.config/bean/env`~~ — đã migrate và xoá trên máy chủ (doctor sạch) |
| **M30 — Updater (D23)** | 🟡 Code + test xanh, chưa phát hành tag thật | (commit này) | Crate `bean-update`: hỏi GitHub Releases (chỉ stable, D23.7), tải + verify SHA256, giải nén chặn tar-slip, ghi đè binary atomic (Unix rename / Windows `.old`), `bean update [--check] [--no-restart]` tự restart qua `beanctl` (D23.5); `GET /api/update/check` admin-only; thẻ “Phiên bản & cập nhật” trong Cài đặt → Nâng cao; workflow `release.yml` build 5 target khi push tag `v*` | Chưa push tag `v*` thật nên chưa chứng minh end-to-end workflow release; D23.3 chưa có chữ ký GPG (đã chấp nhận rủi ro) |

**Chú thích:** “🟡” không có nghĩa code chưa tồn tại; đó là trường hợp logic đã triển khai và test xanh nhưng tiêu chí vận hành thực tế hoặc known issue liên quan chưa hoàn tất.

## 3. Năng lực đã triển khai

### Lõi agent

- Một binary Rust với CLI, Web server và Telegram trong cùng tiến trình.
- `Router` sở hữu run; mất WebSocket không huỷ run.
- Hàng đời theo session, cancel tường minh, confirm có timeout.
- Lỗi tool trở thành tool result thay vì làm hỏng loop.
- Giới hạn bước, ngân sách token/ngày, chống lặp tool, audit JSONL.
- Kế hoạch có cấu trúc: tool `update_plan` (Safe) — model gửi snapshot toàn bộ danh sách bước, UI web hiện thẻ tiến độ lúc đang chạy; plan sống theo run, khôi phục qua `Sync` (lộ trình mục 5b, D25, 2026-11-10).
- Checkpoint/rollback workspace: chụp snapshot vào `data.dir/checkpoints/` trước tool ghi đầu tiên của lượt (run thuần đọc không tốn chi phí); tool `rollback_workspace` (Confirm, kèm danh sách thay đổi thật trong prompt xác nhận) hoàn tác mọi thay đổi của lượt hiện tại; capture fail ⇒ từ chối tool ghi (lộ trình mục 5c, D26, 2026-11-11).
- Hiển thị diff khi duyệt ghi file: `write_file`/`edit_file` kèm diff thật đo tại thời điểm hỏi trong prompt xác nhận (web tô màu +/−), thay preview `content=<toàn bộ>`; `edit_file` cảnh báo trước khi `old` không khớp duy nhất (D27, 2026-11-11). **Đã kiểm chứng tay qua web với model thật**: thẻ hiển thị đúng `ghi đè (9 dòng → 9 dòng)`, diff 2 thay đổi (ngày + ghi chú), context hai phía nguyên vẹn.

### Provider và tool

- Anthropic Messages API.
- OpenAI-compatible, hỗ trợ Ollama/vLLM khi có `base_url`.
- Streaming SSE cho cả hai provider.
- Failover provider: `[llm].fallback_profiles` — provider chính lỗi 429/5xx/mạng hoặc 401/403 sau khi hết retry thì thử lần lượt profile dự phòng (lộ trình mục 4, 2026-11-10).
- Model phụ `[llm].small_model` cho tác vụ rẻ (compaction, reflection); prompt caching `[llm].prompt_cache` cho Anthropic.
- Tool song song: các tool `Safe` liên tiếp trong cùng lượt model chạy đồng thời (trần 4, semaphore); `Confirm`/`Dangerous` luôn tuần tự; kill-switch `[agent].parallel_safe_tools`. Context chuẩn hoá thứ tự tool result theo `tool_use` (lộ trình mục 5a, D24, 2026-11-10).
- File tools, glob, grep, shell sandbox, web fetch/search, memory, skills, scheduler.
- MCP client stdio.

### Bộ nhớ và skills

- SQLite + FTS5, lịch sử bền vững.
- Context budget và compaction không cắt sai cặp tool.
- Truy xuất: `memory_search` loại message của lượt đang chạy khỏi kết quả (K5); context builder đọc lịch sử **theo trang** từ mới nhất lùi về cũ, dừng khi đủ ngân sách — không đọc toàn bộ mỗi bước (K2) (lộ trình mục 6, 2026-11-11).
- `MEMORY.md`, `USER.md`, memory entries.
- Skills từ project/user directory; skill draft chỉ kích hoạt sau duyệt; reflection lưu candidate ở trạng thái chờ, chỉ thành draft khi lượt kế tiếp của cùng phiên không phản hồi tiêu cực (K16, D28, 2026-11-13).

### Web UI

- Login, chat, tool/confirm cards, Stop.
- Quản lý session, memory files, memories, skills, tasks, audit, status.
- WebSocket reconnect với `Sync`.
- Streaming token hiện dần.
- Markdown render an toàn; không raw HTML và không remote image.

### Vận hành

- Structured JSON logs, secret redaction.
- Graceful shutdown.
- systemd unit và Dockerfile nhiều tầng.
- Make targets: `check`, `audit`, `build`, `build-headless`, `e2e`, `smoke-scheduler`.

## 4. Bằng chứng kiểm thử

| Hạng mục | Kết quả gần nhất | Ghi chú |
|---|---|---|
| `make check` | ✅ Pass | `cargo fmt`, `clippy -D warnings`, toàn bộ test Rust, type export, Biome, `tsc`, Vitest và web production build |
| Test Rust | ✅ 319 test pass | 43 test binary trong lần `cargo test --workspace` gần nhất; type export chạy riêng 42 test binding |
| Test web | ✅ 31 test pass | 12 test file: chat store/WS, confirm card, safe markdown, các trang quản lý, i18n và auth redirect |
| `make audit` | ✅ Pass | RustSec scan `Cargo.lock` và `pnpm audit --prod`; không có vulnerability được báo |
| `make e2e` | ✅ Pass | Binary `serve --fake-llm`: login → chat → confirm → Stop → reconnect `Sync`; thêm test Telegram allowlist |
| `make smoke-scheduler` | ✅ Pass | Một giờ ảo với fast tick |
| `make build` | ✅ Pass | Web build + Rust release có UI nhúng |
| `make build-headless` | ✅ Pass | Release không UI, không cần Node |
| Playwright | ⬜ Chưa có | Là tuỳ chọn trong M16 |
| CI | ✅ Xanh 4/4 trên `59efd92` (2026-11-09) | `ci.yml`: check-rust / check-web / eval baseline (fake) / audit. Đã xử lý hai sự cố trong ngày: (1) `cargo audit` báo 4 advisory mới (rmcp 3.4.1 ×3 + libsqlite3-sys) → nâng `rmcp 3.9.0` + `rusqlite 0.41.1` (commit `fix(deps)` `59efd92`); (2) một run fail job check-rust vì test flaky chưa xác định được (pass ở run kế tiếp — xem known-issues mục 4). Flake flaky lặp lại lần 2 trên `d4d72f7` (2026-11-10, local xanh): `ci.yml` giờ in dòng test fail ra **annotations** để lần kế chẩn đoán được mà không cần đăng nhập đọc log |
| Eval provider thật (K12/K17) | ✅ Đã chạy opt-in với model thật | `bean eval run --tasks evals/tasks-real --provider real` (key từ env, không vào CI): K12 compaction pass (OpenRouter nemotron, ~3,5K/0,7K token); K17 reflection pass (GLM-5.3-flash: 6 tool call → JSON đúng schema → draft tạo được). Model free flaky đã ghi lại trong K17 |
| K15 Telegram thật | ✅ Smoke tay trên bot thật | Chat end-to-end, confirm inline keyboard (audit `decided_by: telegram:8845250886`), Dangerous không nút phiên, `/stop`, reconnect 2 lần, đồng thời web — chi tiết `docs/known-issues.md` K15 |
| Failover + model phụ + prompt cache | ✅ Đã kiểm chứng với model thật | Task eval `real_failover_dead_primary` (opt-in, key từ env): primary chết chắc (cổng 9) — 3 lần retry ~9s → log "chuyển sang provider dự phòng" → fallback NVIDIA GLM-5.3-flash trả lời bằng model riêng của profile nó → run kết thúc `final` (1599/52 token). Bug thật bắt được trong lần chạy đầu: fallback nhận model override của primary (`dead-model`) → 404; đã sửa — override chỉ áp cho primary, fallback luôn chạy model riêng (test `fallback_ignores_model_override`). Máy đang chạy đã bật `fallback_profiles = ["open-router"]`, restart `beanctl` sạch, banner CLI `provider: failover` |
| 7 unit test `FailoverProvider` + wire cache Anthropic | ✅ Pass | 503/401 failover, Decode không failover, hết đường trả lỗi cuối, stream trước delta đầu, fallback bỏ qua model override; `system` có `cache_control: ephemeral` khi bật, string khi tắt; reflection dùng `small_model` (test ghi lại model) |
| Tool song song (mục 5a lộ trình) | ✅ Pass | 171 test bean-core: đo `max_active` chứng minh 2 Safe chạy đồng thời; kill-switch chạy tuần lại; batch trộn Safe+Confirm chỉ hỏi 1 xác nhận; 4 test phân nhóm (`partition_tool_calls`); 3 test chuẩn hoá thứ tự; test cancel giữ cặp theo ngữ nghĩa mới. Bug thật bắt được: huỷ bị đếm như lỗi tool → gợi ý "thất bại 2 lần" gắn sai vào result huỷ (D24.5, đã sửa) |
| Kế hoạch có cấu trúc (mục 5b lộ trình) | ✅ Pass | `bean-tools` 40 test (5 plan: schema+ràng buộc → None, trần 200 ký tự theo char, tóm tắt đúng); `agent_loop` 21 test (2 mới: plan hợp lệ phát `on_plan` + giữ cặp tool_use/tool_result; args sai không phát). Bug thật bắt được: `parse_plan_steps` bản đầu chỉ kiểm schema, `steps: []` lọt qua được phát ra UI — gộp `validate_steps` dùng chung cho loop và tool (D25.2). Web 233 test (chat store plan: ghi đè/final xoá, dedupe, `applySync` khôi phục; PlanCard) |
| Checkpoint/rollback workspace (mục 5c lộ trình) | ✅ Pass | `bean-security` 5 test (chụp/restore/trần/prune + proptest cây ngẫu nhiên); `bean-core` 4 test `checkpoint_flow` (rollback về trước khi ghi; chưa có gì thay đổi; checkpoint tắt ⇒ tool ghi vẫn chạy; scheduler từ chối); E2E binary thật `rollback_restores_workspace_in_real_binary` (duyệt 2 confirm, file sinh sau checkpoint bị xoá, snapshot ngoài jail, result qua REST đúng). Chi tiết D26.1–D26.9. **Còn kiểm chứng tay với model thật** qua Telegram/web (ghi file xong bảo hoàn tác) — người dùng sẽ thử |
| Truy xuất bộ nhớ K5 + K2 (mục 6 lộ trình) | ✅ Pass | K5: `memory_search_bo_message_cua_luot_dang_chay` (sqlite) — message của lượt đang chạy bị loại, lịch sử lượt trước + session khác vẫn thấy, gọi ngoài lượt vẫn thấy đủ. K2: `build_windowed_history_equals_full_history_trim` (250 message, nhóm tool đúng biên trang 200 — windowed ≡ full-trim) + `build_with_large_budget_returns_whole_history_with_tool_group_intact` (nhóm tool vắt biên trang giữ nguyên thứ tự chuẩn hoá) |
| Diff duyệt hành động (D27) | ✅ Pass | `bean-tools` 9 test `diff.rs` (context thật + số dòng chuẩn, trần 50 dòng + note, fallback file quá lớn không treo, tiếng Việt + emoji, chỉ khác newline) + 9 test `confirm_details` (fake `WorkspaceFs`: tạo mới/ghi đè/nhị phân/lỗi I/O/thiếu args/old không duy nhất/wire qua tool thật) + `bean-core` 2 integration test (prompt chứa `+dòng mới`, không còn `content=`; cảnh báo old xuất hiện 2 lần) + web 3 test `ConfirmCard` (màu +/−/header, mô tả trước diff không tô, không diff không sinh span). Bug thật bắt được: bản đầu LCS chạy trên đoạn giữa sau trim ⇒ hunk mất context hai phía — giữ prefix/suffix dạng `Equal` trong dãy op |
| Reflection trì hoãn K16 (D28) | ✅ Pass | `bean-skills` 10 test (candidate FIFO theo phiên, sống qua reload, validate rẻ + thăng hạng, hồi quy không ghi file khi state rỗng — bug thật: `make check` ghi bẩn `crates/bean/skills/`); `bean-core` learning 12/12 (complaint bỏ vĩnh viễn + audit, sống qua rebuild Router, slash command không promote, luồng 2 lượt promote sau phản hồi trung tính); eval fake/real thêm `followup_prompt`; E2E mở rộng (run 1 không Notification, run 2 Notification trước Final) |

### Ghi chú về warning dependency

`make check`/`cargo test` có warning future-incompatibility:

```text
proc-macro-error2 v2.0.1
```

Dependency chain: `teloxide 0.17.0` → `aquamarine 0.6.0` → `proc-macro-error2 2.0.1`. Đây là proc-macro build-time warning `E0365`, không phải lỗ hổng runtime; đã ghi tại K21. `make audit` vẫn xanh với ngoại lệ unmaintained `RUSTSEC-2026-0173` của `aquamarine` được khai báo tường minh trong Makefile.

## 5. Trạng thái local trên máy hiện tại

- **Soak 24h đang chạy (lộ trình chất lượng — đề xuất #1):** bắt đầu **2026-11-12 17:42 ICT**
  sau `make build` + `scripts/beanctl restart` với binary tại commit `790d297`
  (mang K6 + K33 + K22; docs `1156f11`). Instance pid ghi lúc start: `37541`,
  provider NVIDIA GLM-5.3-flash qua `openai_compat`, web `0.0.0.0:7878`, Telegram bật.
  Rà `/tmp/bean-run/serve.log` (lỗi, restart, memory, WS reconnect) khi hết 24 giờ
  (**2026-11-13 ~17:42 ICT**) rồi cập nhật mục 6.2.5.

- Có binary release tại `target/release/bean`.
- Đã tạo `~/.bean/auth.toml` với quyền `0600`.
- ~~**Chưa có `bean.toml`**~~ — **cập nhật 2026-11-02:** đã có, kèm RBAC (`[agent.user_roles]` +
  4 role). Provider đang dùng là OpenRouter qua `openai_compat`; key qua UI/env. Hai file trạng
  thái runtime `llm-state.json`/`voice-state.json` **thắng** `bean.toml` lúc khởi động.
- `auth.toml` đã có, nên bước xác thực Web đã sẵn sàng.
- Để chạy fake/demo không cần key:

```bash
./target/release/bean serve --fake-llm tests/e2e/demo_hello.json
```

- Để chạy thật:

```bash
export ANTHROPIC_API_KEY='...'
./target/release/bean serve
```

- Để tránh phụ thuộc key mặc định và cấu hình đầy đủ, nên copy file mẫu trước:

```bash
cp bean.example.toml bean.toml
```

## 6. Các phần chưa hoàn thành

### 6.1. Chưa triển khai

1. **Kênh chat thứ ba (M18 Discord).** — **ĐÃ LOẠI BỎ, không phải việc tồn đọng.**
   Chủ dự án quyết bỏ 2026-09-26. Telegram + web là bộ kênh cuối cùng; xem `Plan.md` mục 5.0.

2. **CI tự động.** — **ĐÃ XONG (2026-11-09).**
   - `.github/workflows/ci.yml` (check-rust, check-web, eval baseline fake, audit — trigger push/PR) chạy xanh toàn bộ trên GitHub Actions từ lần chạy đầu; `release.yml` từ M30 cũng xanh khi push tag.
   - `make check`, `make audit`, `make e2e` ngoài CI vẫn là cổng local của mỗi milestone.

3. **Playwright/browser E2E.**
   - Đây là tuỳ chọn của M16, chưa có.
   - UI hiện mới được kiểm thử bằng Vitest/Testing Library/MSW và WebSocket client test.

4. **Triển khai production thực tế.**
   - `systemd` unit và Dockerfile đã có nhưng chưa được cài/chạy trên host production.
   - Chưa có bằng chứng image production đã build/push/deploy.
   - Chưa có quy trình backup/restore được kiểm thử trên môi trường thật.

### 6.2. Đã có code nhưng chưa kiểm chứng thực tế

1. **Provider thật (K12/K17).** — **compaction và reflection đã kiểm chứng với model thật (2026-11-09)** qua khung eval opt-in `evals/tasks-real/`:
   - K12: `real_compaction_long_history` (OpenRouter, nemotron free) — hội thoại đủ dài kích hoạt compaction, summary lưu, run sạch; số đo tại `evals/out/metrics.jsonl`.
   - K17: `real_reflection_skill_draft` (GLM-5.3-flash) — 6 tool call thật, reflection JSON đúng schema, draft được tạo và pass assert; model free flaky (chi tiết trong `docs/known-issues.md` K17). Đã có phương án giảm flaky: `[llm].fallback_profiles` (2026-11-10) — khai báo profile dự phòng để run tự chuyển khi model free trả 503/401.
   - Còn lại: streaming thật (M17) và biến thể Anthropic/Ollama — chưa có key tương ứng. Prompt caching Anthropic (2026-11-10) chỉ verify bằng wire test vì cùng lý do.

2. **Telegram bot thật (K15).** — **ĐÃ ĐÓNG (2026-11-10): smoke test tay trên bot thật pass trọn checklist tương tác.**
   - Chat thường end-to-end (session 19: "chào bean" → trả lời 4s); tool `Confirm`/`Dangerous` duyệt qua inline keyboard, audit ghi đúng actor `telegram:8845250886`; `/stop` huỷ run giữa chừng sạch; reconnect qua 2 lần restart; web + Telegram một instance.
   - Còn lại thuộc soak 24h (mục 5) — không phải K15. Callback người lạ chỉ unit test (cần tài khoản thứ hai để smoke tay).

3. **Scheduler thật (M13).**
   - Có test fake clock và smoke một giờ ảo.
   - Chưa chứng minh tác vụ “7h mỗi sáng” thật sự gửi qua Telegram và xuất hiện trong UI trên máy đang chạy.

4. **MCP bên thứ ba (M14).**
   - Đã kết nối MCP stdio server thật trong test process.
   - Chưa kiểm thử một MCP server bên ngoài, chưa ghi tài liệu checklist trust/reconnect cụ thể theo từng server.

5. **Soak test 24 giờ (M16).**
   - Chưa có báo cáo chạy liên tục 24 giờ.
   - Vì vậy chưa đủ cơ sở tuyên bố production-ready hoặc đánh giá rò rỉ memory/connection/task leak.

### 6.3. Backlog kỹ thuật từ `docs/known-issues.md`

| Nhóm | Mục | Mức | Tóm tắt |
|---|---|---:|---|
| An toàn | S1 | đã đóng | `read_file`/`grep`/`glob`/`list_dir`/output `run_shell` đã bọc `<untrusted_content>` và bật `untrusted_seen`; thêm `Tool::marks_untrusted()` + test hồi quy toàn registry — xem `docs/known-issues.md` S1, `docs/decisions.md` D9.1 |
| An toàn | K1 | **đã đóng** (2026-09-27) | `sessions.summary` đã bọc `<untrusted_content>` **và** bật `untrusted_seen` khi context có summary (`TurnContext::summary_present`). Đánh đổi đã được duyệt: phiên đã compact mất "cho phép trong phiên" cho `Confirm`/`Dangerous` tới hết phiên — xem `docs/decisions.md` D9.7 |
| An toàn | S2 | **đã đóng** (2026-09-27) | Lọc trọn chuỗi ESC/OSC cho output của mọi tool `marks_untrusted()` trước khi in ở CLI; tool không untrusted giữ nguyên hành vi — xem `docs/decisions.md` D9.8 |
| An toàn | S3, S4 | thông tin | `/api/audit` chưa lọc theo người dùng (chấp nhận được khi v1 một người dùng); `allowed_tools` cấp quyền vĩnh viễn — UI nên cảnh báo khi task có tool `Dangerous` |
| An toàn | K1-followup | thấp | Cân nhắc chỉ bật cờ khi summary thực sự tổng hợp từ nội dung untrusted; **chưa làm ngay**, cần dữ liệu dùng thật |
| Độ bền DB | K2–K4 | **đã đóng** (2026-11-11/12) | `load_history_within_budget` đọc theo trang trong ngân sách; store có trần 10s; tách connection chỉ đọc — FTS nặng không còn chặn `append` |
| Store semantics | K5, K6 | **đã đóng** (2026-11-11/12) | `memory_search` loại message của lượt đang chạy; `clear` xoá cả `summary` ở cả hai store, test hành vi chung; K7 (MemoryStore/SqliteStore lệch) vẫn mở |
| Search | K8–K9 | thấp | BM25 chỉ tương đối theo nguồn; cú pháp FTS bị giới hạn |
| E2E | K11 | **đã đóng** (2026-11-12) | `memory_survives_restart_in_real_binary` trong `make e2e`: memory_save → SIGTERM → restart cùng `data.dir` → memory_search thấy ghi nhớ, phiên + DB còn nguyên |
| Provider thật | K12, K17 | **đã đóng** (2026-11-09) | Compaction + reflection đã chạy với model thật qua khung eval opt-in (`evals/tasks-real/`, key từ env); còn streaming thật (M17) và Anthropic/Ollama — thiếu key |
| Telegram | K15 | **đã đóng** (2026-11-10) | Smoke bot thật pass: chat, confirm inline (audit đúng actor), Dangerous, `/stop`, reconnect, đồng thời web; callback người lạ unit-tested |
| Learning | K20 | thấp | Scheduler draft chưa có inline approval Telegram. K19 đã đóng 2026-11-12: duyệt/bỏ draft ghi audit JSONL với actor, cả lần thử lỗi. **K16 đã đóng 2026-11-13:** reflection lưu candidate trạng thái chờ, lượt kế tiếp cùng phiên không phàn nàn mới thăng hạng thành draft; phản hồi tiêu cực bỏ vĩnh viễn + audit `learning_candidate_discard` (D28) |
| Learning E2E | K18 | **đã đóng** (2026-11-12) | `learning_draft_approval_roundtrip_in_real_binary` trong `make e2e`: 5 tool call → notification WS → draft qua REST → catalog chưa thấy trước duyệt → approve → audit JSONL → run 2 nạp được skill |
| Dependency | K21 | thấp | `proc-macro-error2` future-incompatibility warning qua `aquamarine`/`teloxide` |

**Lưu ý:** K13 (`/new` chỉ ở adapter) và K14 (chưa có API/UI memory files) trong tài liệu backlog đã cũ so với code hiện tại: `/new` đã được xử lý trong Router và UI đã có `GET/PUT /api/memory/files/:name`. Hai mục này nên được đóng hoặc cập nhật trong một đợt tài liệu riêng.

## 7. Khoảng cách so với đặc tả v1

### Đã đạt

- Một binary Rust, React chỉ là build artifact được nhúng.
- CLI, Web và Telegram dùng chung Router/store.
- Agent loop, tool registry, memory, skills, scheduler, MCP và learning loop.
- Path jail, sandbox, confirm policy, audit, untrusted content, SSRF protection.
- Auth Web, REST, WebSocket, UI quản lý và i18n.
- Streaming token M17(A).

### Chưa đạt tiêu chí “vận hành thật”

- Provider thật với hội thoại dài.
- Telegram bot thật.
- Scheduler thật với Telegram/UI.
- MCP server bên thứ ba.
- Soak test 24 giờ.
- CI và deploy production.
- Playwright/browser E2E.


## 8. Ngoài phạm vi v1

Các nhóm sau không phải milestone đang dang dở và không nên tính là “chưa hoàn thành” nếu triển khai v1:

> **Cập nhật 2026-11-02:** voice (M27) và điều khiển trình duyệt (M26) **đã vào scope và triển
> khai** — không còn "ngoài phạm vi". Các "agent con" (M21–M24) là **role trong cùng một tiến
> trình**, không phải multi-agent song song (vẫn backlog, `Plan.md` mục 6).

- Multi-user/SaaS, ứng dụng mobile native, fine-tuning.
- Sub-agent song song (kiểu Hermes), kiến trúc B nhiều tiến trình — backlog.
- Kênh chat khác ngoài web + Telegram (WhatsApp/Signal/Discord — bộ kênh cuối đã chốt 2026-09-26).
- Chỉnh sửa hoặc tạo lại tin nhắn trong UI.
- TLS trực tiếp và các tính năng vận hành quy mô lớn.

## 9. Khuyến nghị thứ tự tiếp theo

1. **An toàn:** S1, K1, S2 đã đóng. Việc còn lại là rà S3/S4 khi chuyển multi-user và khi
   thêm cảnh báo UI cho task `Dangerous`; cân nhắc `K1-followup` nếu dùng thật thấy mỏi tay.
2. **Vận hành tối thiểu:** tạo `bean.toml`, đặt secret qua biến môi trường, chạy một smoke test provider thật và một smoke test Telegram thật.
3. **E2E bền vững:** đóng gói K11 và K18; kiểm tra restart, scheduler, learning draft và skill activation.
4. **Độ bền:** xử lý hoặc ghi nhận rõ K2–K7 trước khi có phiên dài và DB lớn.
5. **Phát hành:** thêm CI, soak test 24 giờ, Playwright tùy chọn, cài systemd/Docker trên host thật và diễn tập backup/restore.
6. **Kênh chat:** không còn việc mở thêm kênh. M18 (Discord) đã bị loại bỏ 2026-09-26.

## 10. Tiêu chí có thể gọi là “v1 hoàn thành”

- [x] M1–M16 có mã nguồn và commit tương ứng.
- [x] `make check` xanh.
- [x] `make audit` xanh với ngoại lệ được ghi rõ.
- [x] `make e2e` và scheduler smoke xanh.
- [x] Release có UI nhúng và headless build thành công.
- [x] M17(A) streaming token hoàn thành.
- [x] Provider thật đã smoke test (OpenRouter qua `openai_compat`, 2026-11-02).
- [x] Telegram bot thật đã smoke test (K15, 2026-11-10 — chat, confirm, `/stop`, reconnect).
- [ ] Scheduler thật đã kiểm chứng.
- [ ] MCP server bên thứ ba đã kiểm thử.
- [x] S1 đã xử lý (2026-09-26).
- [x] K1 đã xử lý (2026-09-27) — bọc `<untrusted_content>` + bật `untrusted_seen` khi có summary.
- [x] S2 đã xử lý (2026-09-27) — lọc escape terminal cho output tool untrusted ở CLI.
- [ ] E2E memory và learning đầy đủ.
- [ ] Soak test 24 giờ.
- [ ] CI và quy trình deploy/backup production.

## 11. Kết luận

Bean hiện là một **bản v1 chức năng phong phú, tự kiểm thử tốt và có thể chạy demo bằng FakeProvider**. M17(A) đã hoàn thành. Rủi ro lớn nhất hiện nay không nằm ở thiếu chức năng chính, mà ở việc chưa kiểm chứng hệ thống thật: provider dài hạn, Telegram thật (K15), E2E bền vững, soak test và production deployment (K1 đã đóng 2026-09-27).
