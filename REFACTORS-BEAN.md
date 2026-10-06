# Ghi chú refactor Bean

Ghi lại **vì sao** mỗi lần tách, để người sau (kể cả tôi ở phiên mới) hiểu đường cắt
đến từ đâu — vì tách file thủ công thì **lý do** quan trọng hơn kết quả.

Quy ước: mỗi mục ghi vấn đề đo được → cách cắt → cách kiểm chứng.

---

## Đo trước, đừng đoán

Câu hỏi thường gặp khi thấy file dài: *"Rust phải viết dài nên vậy đúng không?"*

**Câu trả lời: không.** Đo thật trong repo này (`store.rs` lúc đầu):

| File | Tổng | Trống | Comment | Code thật |
|---|---|---|---|---|
| `bean-memory/store.rs` | 4879 | 321 | 327 | **4231** |
| `bean-types/config.rs` | 2372 | 137 | 656 | **1579** |
| `bean-core/router.rs` | 1349 | 79 | 230 | **1040** |

Comment chỉ chiếm **6,7%** file lớn nhất — không phải file bị bơm bằng chú thích.

Nguyên nhân thật của `store.rs`: nó chứa **ba bản của cùng một danh sách 51 thao tác**
(khai báo `trait Store` + `impl Store for MemoryStore` + `impl Store for SqliteStore`),
cộng ~1100 dòng truy vấn SQL và 579 dòng test. Chỗ bị trả giá thật vì Rust là bước
**đóng gói lệnh qua kênh** tới worker thread (`rusqlite` là API blocking, agents.md
mục 22.8) — mỗi method 1 dòng logic hoá thành ~8 dòng khai báo lệnh + reply.

**Bài học rút ra:** số dòng lớn là *triệu chứng*. Nguyên nhân phải đo mới tìm ra được.

---

## 1. `bean-memory/store.rs` → 23 module (xong)

**Vấn đề.** File 4879 dòng, lớn nhất repo, gấp 3,6 lần `router.rs` — nhưng **không có
guardrail nào** canh, trong khi `router.rs` đã có `tests/architecture.rs`. Đây là rủi ro
khó bảo trì nhất: *file lớn nhất lại ít được bảo vệ nhất*.

**Cách cắt** — theo **ranh giới nghiệp vụ**, không chia đều số dòng:

```text
store/
├─ trait_def.rs    trait Store — bề mặt ổn định mà lớp trên chỉ cần biết
├─ types.rs        kiểu record + StoreError (API công khai của crate)
├─ budget.rs       ngân sách token (chính sách tầng trên, phụ thuộc store)
├─ shared.rs       hàm hai bản cài đặt PHẢI dùng chung
├─ compaction.rs   quy trình nén lịch sử — bất biến "không cắt đôi cặp tool"
├─ memory.rs       MemoryStore (test double)
├─ tests.rs        test dùng chung cho cả hai bản cài đặt
└─ sqlite/         schema, command, worker, migration + 7 nhóm truy vấn `q_*.rs`
```

Kết quả: file lớn nhất **4879 → 925** dòng (−81%). Không đổi logic nào.

**Ba điều chỉnh thiết kế phát sinh trong lúc tách:**

1. `now_rfc3339` phải ở `shared.rs`, **không** phải trong `sqlite/`. Cả hai bản cài đặt
   đều ghi timestamp và so sánh bằng **chuỗi** trong SQL (`next_attempt_at`). Để trong
   `sqlite/` thì `MemoryStore` phải import ngược lên module của bản SQLite — sai hướng
   phụ thuộc. Đã có test canh "tồn tại đúng một chỗ".

2. `impl Store for SqliteStore` (547 dòng) **giữ nguyên một khối, không tách**. Đây là
   *bảng tra cứu* chứ không phải logic: tách ra 8 file sẽ **làm giảm** khả năng kiểm
   chứng, vì không ai đọc rời 8 file để đối chiếu xem đủ 51 method không.

3. Trần dòng phải **riêng cho từng file**, không dùng một con số chung — file `mod.rs`
   89 dòng và `q_session.rs` 258 dòng có bản chất khác nhau.

**Kiểm chứng:** `cargo clippy --workspace -- -D warnings` ✅ · `cargo test --workspace`
✅ · 29/29 test `bean-memory` · 7/7 guardrail mới.

**Tai nạn đáng ghi.** Một lần `sed -i` chạy trên file **rỗng** đã xoá mất 245 dòng
nội dung `q_session.rs`. Chỉ phát hiện được vì compiler báo `no list_message_records`
trong khi các file anh em đều bình thường — tức là lỗi **không im lặng**. Đã khôi phục
từ backup `/tmp/store_backup.rs`.

→ **Bài học:** tách file thủ công thì `sed` là đũa ghi dấu, không phải công cụ kiểm chứng.
Backup trước, và **review `git diff` từng khối** trước khi commit. Lỗi im lặng ở đây
nguy hiểm hơn nhiều lần so với biên dịch hỏng.

---

## 2. `bean-web/server.rs` → 14 module (xong)

**Vấn đề.** File 1897 dòng trộn **bốn** việc không liên quan: middleware bảo mật, ~30
handler REST, vòng lặp WebSocket, và dựng router.

Hậu quả cụ thể: sửa kiểm tra `Origin` (mục 15.7) — **lớp rủi ro nghiêm trọng nhất của
dự án**, vì agent có quyền chạy lệnh nên một lỗ CSRF ở đây là một lỗ thực thi từ xa —
lại buộc phải mở file chứa cả `run_socket` và 30 handler.

**Cách cắt** — theo **ranh giới bảo mật và nghiệp vụ**:

```text
server/
├─ mod.rs (274)             WebState, ApiFailure, WebChannel, bản đồ
├─ guard.rs (122)           Origin/Host/CSRF + security headers  ← lớp phòng thủ
├─ dto.rs (81)              ranh giới record của store → JSON
├─ routes.rs (73)           dựng axum router
├─ ui_assets.rs (75)        UI nhúng + SPA fallback
├─ ws.rs (468)              vòng lặp WebSocket (máy trạng thái)
└─ rest_*.rs                7 nhóm tài nguyên, lớn nhất rest_tasks.rs (218)
```

Kết quả: file lớn nhất **1897 → 468** dòng (−75%).

**Nguyên tắc đặt ra cho file bảo mật.** `guard.rs` phải nằm **riêng**, không lẫn với
handler, vì: một lớp phòng thủ bị xoá trong lúc refactor là mất lớp phòng thủ đó mà
**không ai thấy khi đọc code handler**. Nay có test canh tên cả hai middleware.

**Bốn lỗi cắt sai đã bắt được nhờ test** (đáng ghi vì không lỗi nào là do logic):

1. `list_agents` bị cắt đứt giữa chừng → sang nhầm `rest_sessions.rs`.
2. `usage_history` bị mất hoàn toàn khi tái tạo file.
3. Hằng `DEFAULT_USAGE_DAYS`/`MAX_USAGE_DAYS` rơi lọt sang file kề bên.
4. `rest_sessions.rs` mất header khi viết lại bằng script → mất `use`, mất `State`…

Cả bốn đều bị compiler hoặc test bắt ngay. Nhưng **hai lỗi đầu là do chạy `sed` trên file
đã sửa trước đó** — đúng cái bẫy đã ghi ở mục 1.

**Kiểm chứng:** `cargo clippy --workspace -- -D warnings` ✅ · `cargo test --workspace` ✅ ·
86/86 test `bean-web` (gồm toàn bộ test bảo mật: cookie `Secure`, `Origin` sai → 403,
WS thiếu cookie bị từ chối trước upgrade, `/api/*` 404 JSON, không lộ secret) ·
5/5 guardrail mới · `make types` không lệch.

---

## 3. `bean-channels/telegram.rs` → 10 module (xong)

**Vấn đề.** File 1992 dòng trộn **năm** việc: kiểu dữ liệu, quy tắc giới hạn của kênh,
transport `teloxide`, bảng đích callback, và vòng lặp polling.

Telegram là kênh có **bề mặt tấn công** — allowlist user là điều kiện an toàn, bỏ nó là
biến bot thành tài khoản đọc/ghi tin nhắn của bất kỳ ai. Sửa quy tắc đó lại phải mở file
chứa vòng lặp `teloxide`.

**Cách cắt** — theo **loại công việc**:

```text
telegram/
├─ mod.rs (75)          hằng số, bản đồ, re-export
├─ types.rs (142)       kiểu dữ liệu + trait `TelegramTransport`  ← seam cho test
├─ text_rate.rs (129)   cắt tin an toàn, giới hạn tần suất, chống trùng update
├─ transport.rs (243)   cầu nối teloxide
├─ targets.rs (113)     bảng đích theo run_id + phân tích callback
├─ channel_core.rs (184) dựng TelegramChannel, typing, gửi tin, skill nháp
├─ handlers.rs (198)    tin nhắn + callback quyết định   ← allowlist ở đây
├─ events.rs (146)      RunEvent của Router → tin Telegram
├─ polling.rs (184)     vòng lặp polling + impl Channel
└─ tests.rs (786)       test, không cần mạng
```

File sản phẩm lớn nhất: **1992 → 243** dòng (−88%).

**Hai điều chỉnh phát sinh:**

1. `loop.rs` phải đổi tên thành `polling.rs` — `loop` là **keyword Rust**.
2. Không cắt giữa một `impl`. Ban đầu tôi cắt `impl TelegramChannel` làm 5 khối rời rạc,
   rồi phải đoán số `}` cần thêm — dẫn tới một chuỗi lỗi ngoặc. Cách đúng là **tách theo
   từng method** (dò tìm bằng cân bằng ngoặc), rồi mỗi file một `impl TelegramChannel`
   riêng — Rust cho phép nhiều `impl`, và mỗi file **tự cân bằng** nên không có đoán số nào.

**Guardrail bắt được một lỗi thật.** Sau khi đổi `loop.rs` → `polling.rs`, test
`mod_doc_documents_the_module_tree` **fail** vì doc `mod.rs` vẫn ghi tên cũ. Đây chính là
loại lỗi im lặng mà guardrail sinh ra để bắt — không có test thì ai cũng bỏ qua.

**Một lỗi tôi tự tạo ra:** thêm hằng `CONFIRM_TIMEOUT_SECONDS` vào `mod.rs` với doc
"tránh treo confirm" — hoàn toàn **không tồn tại trong bản gốc**. Clippy báo
`never used` và nó bị xoá. Bài học: khi tách, **không tự thêm thứ mình nghĩ là nên có**;
thêm hằng mới là thay đổi hành vi, phải tách riêng.

**Kiểm chứng:** `clippy --workspace -D warnings` ✅ · `cargo test --workspace` ✅ ·
16/16 test `bean-channels` (gồm `allowlist_blocks_unknown_user_without_reply`,
`callback_from_unknown_user_does_not_resolve_confirm`, `dedup_does_not_submit_same_update_twice`) ·
4/4 guardrail mới.

---

## 4. `bean-types/config.rs` → 14 module (xong)

**Vấn đề.** File 2372 dòng — lớn nhất workspace — trộn 16 section của `bean.toml`
(mục 18), bốn khối `impl Config` (nạp, truy vấn, validate ~700 dòng, resolve secret)
và mọi validator của sản phẩm. Sửa một quy tắc validate phải lội qua file chứa
secret và RBAC.

Config là **nguồn quyết định an toàn**: `validate()` chặn tổ hợp vô nghĩa trước khi
agent chạy, `resolve_*` giữ bí mật không lọt vào prompt. Hai việc đó phải tách bạch.

**Cách cắt** — theo **section `bean.toml`** (mục 18) + theo **trách nhiệm**:

```text
config/
├─ mod.rs (229)               Config, ResolvedSecrets, 16 hằng số, bản đồ, re-export
├─ error.rs (54)              ConfigError, invalid, expand_tilde
├─ enums.rs (117)             enum dùng chung (LlmProviderKind, SandboxMode, ...)
├─ agent.rs (174)             [agent] [roles] [projects] [data]
├─ llm.rs (92)                [llm] [tools.web_search] [learning]
├─ tools.rs (80)              [tools] [security.sandbox] [security]
├─ channels.rs (86)           [web] [telegram]
├─ integrations.rs (176)      [[mcp_servers]] [mcp] [browser]
├─ products.rs (260)          [billing] [scan] [marketing] [qa]
├─ load.rs (73)               load / load_or_default / validate
├─ access.rs (127)            accessor rbac / role / mcp / project
├─ validate.rs (466)          validate lõi + validator free fn
├─ validate_integrations.rs (395) validate tích hợp
└─ secrets.rs (193)           resolve_* từ biến môi trường
```

File sản phẩm lớn nhất: **2372 → 466** dòng (−80%).

**Lần đầu dùng script hóa** (2372 dòng lớn hơn mọi lần trước): Python parse item theo
cột 0 → chia từng method trong `impl Config` → gán vào file theo bảng, fail-loudly khi
có item chưa khớp. Ba bug của chính script, bắt được trước khi xoá bản gốc:

1. Comment/attr dẫn dắt item phải gộp **vào item sau**, không phải item trước.
2. `end` của method k phải là `start` của method k+1 (sau khi lùi qua comment dẫn dắt);
   lấy dòng `fn` của k+1 thì comment bị **nhân đôi** ở hai đầu.
3. Lùi `-1` mù để tìm `}` đóng impl trượt khi block có dòng trắng cuối → thừa `}`.
   Cách đúng: tìm dòng `}` ở cột 0.

**Không đổi hành vi.** `validate()` ở bản gốc gọi `validate_mcp_servers()` **hai lần**
(dòng 1225–1226) — giữ nguyên; tách file là refactor thuần, sửa hành vi là việc khác.

**Visibility học được:** `use super::*` chỉ thấy mục `pub` được `mod.rs` re-export.
Helper nội bộ dùng giữa các module con (`invalid`, `expand_tilde`,
`validate_origin_pattern`, ...) phải là `pub(super)` **và** import tường minh
(`use super::error::invalid;`); method validator của `Config` cũng nâng lên
`pub(super)` vì `load.rs` gọi `validate_core` nằm ở file khác.

**Guardrail mới** `crates/bean-types/tests/architecture.rs` (4 test): trần 600 dòng/file,
tối thiểu 12 module, doc `mod.rs` nhắc đủ 13 file, và re-export đủ hợp đồng API với
các crate khác (`ConfigError`, `TelegramConfig`, `validate_scope_value`, ...).

**Kiểm chứng:** `cargo fmt --check` ✅ · `clippy --workspace -D warnings` ✅ ·
`cargo test --workspace` ✅ **75/75 suite** (thêm 1 suite guardrail) ·
`tests/config.rs` 44/44 · 4/4 guardrail · `make types` không lệch ✅ ·
`pnpm tsc --noEmit` ✅.

---

## Quy trình đã chốt (dùng cho các đợt tách sau)

1. **Khảo sát trước, đo trước** — không đoán số dòng hay số hàm.
2. **Backup file gốc ra `/tmp`** trước khi động vào nó.
3. **Trích xét theo mốc đã xác minh**, không theo ước lượng. Mỗi mốc đọc được dòng
   thật trước khi dùng.
4. **Biên dịch sớm, sửa import từng đợt** — không viết hết rồi mới build.
5. **Test trước khi commit**, tối thiểu: `clippy --workspace -D warnings` +
   `cargo test --workspace` + `cargo fmt --check` + `make types`.
6. **Thêm guardrail** để công việc này không phải làm lại lần nữa.

## Nợ kỹ thuật còn lại (chưa làm, cần đổi chữ ký ở nhiều crate)

- `trait Store` (bean-memory) vẫn gộp 8 nhóm nghiệp vụ — vi phạm ISP.
- `bean-tools/src/registry.rs` giữ workspace + project workspace trong một struct.
- `bean-llm/src/openai_compat.rs` 636 dòng — chưa tách.