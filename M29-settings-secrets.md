# M29 — Thiết lập & secrets thống nhất (UI + terminal)

Tài liệu giao cho coding agent. Đặt cạnh `AGENTS.md`, `Plan.md`, `docs/decisions.md`,
`docs/known-issues.md` trong context. Quy tắc làm việc ở `AGENTS.md` mục 0 áp dụng nguyên vẹn
(đặc biệt: không bịa API, không mở rộng phạm vi, test cùng lúc với code, `make check` xanh).

**Prompt mở đầu để dán:**

> Đọc `M29-settings-secrets.md` và thực hiện milestone M29. **Lập kế hoạch trước, chờ tôi
> duyệt** (danh sách file, crate mới, schema khoá cấu hình, thứ tự làm, các câu hỏi mở ở mục 9)
> rồi mới sửa code. Chỉ làm đúng phạm vi tài liệu này.

---

## 1. Vấn đề cần giải quyết

Hiện trạng: một khoá bí mật có thể nằm ở **bốn nơi** (biến môi trường, `~/.config/bean/env`,
`api_key` trong `bean.toml`, `voice-state.json`) và nơi nào "thắng" phụ thuộc loại khoá. UI ghi
vào file này, terminal chỉ có `bean auth set-password`. Hệ quả đã xảy ra thật: đổi env mà không
có tác dụng vì key trong file che env (K28/Q6), tiến trình khởi động thiếu key (K27).

Mục tiêu: **một nơi lưu, hai cách nhập** (UI và terminal), mọi thứ cùng đi qua một dịch vụ,
giống mô hình của Hermes: secrets tách khỏi cấu hình thường, có wizard `setup`, có lệnh
`config set` tự định tuyến giá trị về đúng nơi.

## 2. Quyết định đã chốt (chủ dự án, 2026-11-02)

| # | Quyết định |
|---|---|
| D19.1 | **Đảo Q6.** Key không còn được ghi vào `bean.toml` hay `voice-state.json`. Toàn bộ secret nằm ở `data.dir/secrets.env`. Q6 giữ nguyên văn trong `known-issues.md`, thêm **Q7** ghi rõ "thay thế Q6" (không viết đè lịch sử). |
| D19.2 | Phạm vi: **tất cả** nhóm thiết lập (LLM/provider, voice, Telegram, web search, billing, marketing, mật khẩu web, cấu hình tĩnh trong `bean.toml`), cả UI lẫn terminal. |
| D19.3 | Thứ tự ưu tiên một chiều: **biến môi trường của tiến trình > `secrets.env` > chưa đặt**. Khi env đang ghi đè, UI/CLI phải nói rõ ("đang bị biến môi trường ghi đè"), không im lặng. |

### Hai quyết định thiết kế do kiến trúc sư đề xuất (coding agent **không được nới lỏng** nếu chưa hỏi lại)

- **D19.4 — Phân tầng khoá cấu hình.** Web session bị chiếm không được trở thành đường nâng
  quyền lên thực thi lệnh. Vì vậy:
  - **Tầng A (sửa được từ UI và CLI):** provider/model, voice, Telegram (bật/tắt, `allowed_user_ids`),
    web search provider, múi giờ, bật/tắt tool, mọi **secret**.
  - **Tầng B (UI chỉ xem; sửa chỉ qua CLI, cần TTY, hỏi xác nhận):** `security.sandbox.*`,
    `web.bind`/`allow_remote`/`public_origin`/`trust_proxy`, `agent.user_roles` + `[[roles]]`,
    `[[infra_scope]]`, `[mcp_server]` + `[[mcp_clients]]`, `browser.allowed_origins`,
    `[qa]`, `agent.workspace`, `data.dir`. UI hiện giá trị hiện tại kèm gợi ý lệnh CLI tương ứng.
  - Danh sách phân tầng nằm trong **một bảng schema duy nhất** (mục 4.2), không rải rác.
- **D19.5 — Step-up cho secret từ UI.** Đặt/xoá secret và đổi mật khẩu từ UI phải nhập lại
  mật khẩu web hiện tại (tái dùng `RateLimiter` của login, D16.11). Phiên web đang đăng nhập
  một mình chưa đủ để thay key hoặc khoá bot.

## 3. Mô hình lưu trữ (đích)

| Loại | Nơi lưu | Ghi chú |
|---|---|---|
| Secret | `data.dir/secrets.env` (`0600`, thư mục `0700`) | Một file duy nhất. UI và CLI cùng ghi |
| Cấu hình tĩnh | `bean.toml` | **Không bao giờ chứa key**. Ghi bằng `toml_edit` (giữ comment) |
| Lựa chọn runtime | `llm-state.json`, `voice-state.json` | Provider/model đang chọn, voice bật/tắt, ngôn ngữ. **Không chứa key** |
| Mật khẩu web | `data.dir/auth.toml` | Hash Argon2id, `0600`, giữ nguyên |
| Token MCP client | Hash SHA-256 trong `bean.db` | Giữ nguyên (M25) |

Mọi nơi hiển thị giá trị phải là **giá trị hiệu lực + nguồn** (`env` | `secrets.env` | `bean.toml`
| `llm-state.json` | `mặc định`). Đây là cách giải quyết bẫy "file thắng toml" (mục 22.24 của AGENTS.md).

### 3.1. Định dạng và an toàn của `secrets.env`

- Dạng `NAME=value`, một dòng một khoá. Giá trị: tối đa 4096 byte, **cấm** xuống dòng/NUL/ký tự
  điều khiển (từ chối, không tự cắt âm thầm).
- **Chỉ chấp nhận tên khoá thuộc "secret slot" đã khai báo** (mục 4.1). Không cho tên tuỳ ý,
  để `secrets.env` không thành đường chèn biến môi trường (`PATH`, `LD_PRELOAD`, `HTTPS_PROXY`…).
- Bean **đọc vào bộ nhớ** (`SecretStore`), **không** `set_var` vào môi trường tiến trình, nên tiến
  trình con (MCP stdio, sandbox, scanner, Chrome) không thừa hưởng. Test hồi quy bắt buộc.
- Ghi **atomic**: tạo file tạm cùng thư mục với quyền `0600` ngay lúc tạo (không `chmod` sau),
  `fsync`, `rename`. Ghi hỏng giữa chừng không được làm mất file cũ.
- Khoá file khi ghi để CLI và server không ghi chồng. Server phát hiện thay đổi từ CLI bằng
  `mtime` khi bắt đầu mỗi run (stat rẻ), không cần watcher.
- `Debug`/`Display`/`tracing`/audit/lỗi/`/api` **không bao giờ** chứa giá trị secret.
  Dùng `secrecy::SecretString` xuyên suốt → **đóng K30**.

## 4. Thành phần

### 4.1. `SecretSlot` registry

Mỗi secret có một slot: `{ name, nhãn, nhóm, dùng bởi }`. Slot **dẫn xuất từ cấu hình**, không hardcode
rải rác: `llm.api_key_env`, `[[llm.providers]].api_key_env`, `tools.web_search.api_key_env`,
`telegram.token_env`, `billing.api_key_env`, các biến credential của `[marketing]`,
`ELEVENLABS_KEY` (voice). `validate()` hiện có (D13.2, D15.3) tiếp tục cấm trùng tên giữa
LLM/search/Telegram/billing/marketing/voice — **không được nới**.

Khi người dùng dán key cho một profile LLM mới từ UI: dùng `api_key_env` của profile; nếu chưa có,
sinh tên `LLM_<TÊN_PROFILE>_API_KEY` (chuẩn hoá `[A-Z0-9_]`), ghi tên đó vào `bean.toml`, ghi giá
trị vào `secrets.env`.

### 4.2. `SettingsSchema` (một bảng duy nhất)

Mỗi khoá cấu hình có: `đường dẫn` (ví dụ `llm.model`), `kiểu`, `tầng` (A/B), `áp dụng` (`hot` /
`restart`), `nơi lưu` (`bean.toml` / `llm-state.json` / `voice-state.json`), `mô tả` (vi/en).
Dùng để: validate `config set`, dựng UI, sinh `bean config list`, quyết định có cần TTY không.
Khoá chưa có trong schema thì **từ chối** `config set` (không ghi tuỳ ý vào TOML).

### 4.3. `SettingsService`

- Đặt trong **crate mới `bean-settings`** (phụ thuộc `bean-types`, `bean-security`; **không** phụ
  thuộc `bean-core`/Router) vì CLI phải dùng được khi chưa khởi động Router. Nếu thấy chỗ đặt tốt
  hơn, nêu lý do và hỏi (quy tắc 4).
- `bean-web` và `bean` (CLI) chỉ là adapter mỏng gọi service này. **Không** có logic thiết lập nào
  khác trong handler hay trong `main` (SRP/DIP, `AGENTS.md` mục 0.10).
- Trước khi ghi `bean.toml`: dựng cấu hình mới → `Config::validate()` → chỉ ghi khi hợp lệ.
  Cấu hình sai bị từ chối ngay, không để lần khởi động sau mới chết.
- Mọi lần ghi: audit `{actor, kênh, khoá, nguồn cũ→mới}`, **không có giá trị**.
- `workspace` không được trùng hoặc chứa `data.dir` (thêm vào `validate()`), để `read_file` của
  agent không với tới `secrets.env`. Có test.

### 4.4. Áp dụng thay đổi

- Khoá `hot` (key LLM, provider/model, voice, search, Telegram allowlist): áp dụng cho **run kế tiếp**.
- Khoá `restart` (bind, origin, role, sandbox…): ghi được, UI/CLI hiện banner "cần `beanctl restart`"
  (so snapshot cấu hình đang chạy với file).
- Telegram: đổi token cần restart kênh; nêu rõ trong kế hoạch cách làm (hot-restart kênh hay bắt buộc
  restart), **hỏi nếu chọn hot-restart** vì liên quan bất biến "một token/instance".

## 5. Terminal (CLI)

```
bean setup                       wizard tương tác, chạy lại được (idempotent)
bean secret set <NAME>           đọc giá trị ẩn trên TTY; có --stdin để pipe. KHÔNG nhận qua argv
bean secret list                 tên + trạng thái + nguồn (+ "bị che bởi env"); không bao giờ in giá trị
bean secret unset <NAME>
bean config get|set|unset|list   route vào đúng nơi theo SettingsSchema; luôn in giá trị hiệu lực + nguồn
bean model                       chọn provider/model tương tác (ghi cùng nơi với UI)
bean auth set-password           giữ nguyên; xem câu hỏi mở 9.1
bean doctor [--online]           kiểm tra sức khoẻ cấu hình (mục 7)
bean config migrate-secrets      chuyển key cũ sang secrets.env (mục 6)
```

Yêu cầu:

- Không bao giờ nhận secret qua tham số dòng lệnh (lọt shell history và `ps`). `secret set` từ chối
  nếu stdin là TTY mà không ẩn được ký tự; kiểm tra crate đọc mật khẩu còn được bảo trì trước khi dùng
  (quy tắc 3).
- `config set` cho khoá **tầng B**: bắt buộc TTY, in giá trị cũ/mới và hỏi `y/N`.
- `bean setup` các bước: provider + key (kiểm tra kết nối), mật khẩu web, (tuỳ chọn) Telegram,
  (tuỳ chọn) web search, (tuỳ chọn) voice. Mỗi bước bỏ qua được. Cuối cùng chạy `doctor`.
- Mã thoát: `0` ok, `1` lỗi, `2` cảnh báo (cho `doctor`), để script dùng được.
- `scripts/beanctl` **thôi nạp** `~/.config/bean/env` sau khi đã migrate (giữ tương thích có cảnh báo
  ở `doctor` nếu file đó còn tồn tại).

## 6. Migrate từ cấu hình cũ

`bean config migrate-secrets` — **dry-run mặc định**, `--apply` mới ghi:

1. Tìm key ở: `[llm].api_key`, `[[llm.providers]].api_key`, key trong `voice-state.json`, và (tuỳ chọn
   `--from-env-file`) `~/.config/bean/env`.
2. Ghi vào `secrets.env` (nếu slot đã có giá trị khác thì **hỏi**, không ghi đè im lặng).
3. Sao lưu file gốc thành `*.pre-m29` (quyền `0600`), rồi xoá key khỏi file gốc.
4. Kiểm chứng sau khi ghi (nạp lại, slot resolve được). Chỉ in **tên** slot, không in giá trị.
5. Nhắc rotate key nếu file từng bị copy/chia sẻ.

Trường `api_key` trong `LlmConfig`/provider/`VoiceConfig` **giữ lại ở chế độ chỉ đọc, deprecated**
(vì `deny_unknown_fields` sẽ làm file cũ không nạp được): đọc được kèm cảnh báo, **không bao giờ
ghi**. Việc gỡ hẳn để milestone sau.

## 7. `bean doctor`

Không bao giờ in giá trị secret. Kiểm tra tối thiểu:

- Quyền: `data.dir` 0700; `secrets.env`, `auth.toml`, `bean.toml`, `*-state.json` 0600; cảnh báo nếu rộng hơn.
- Key sai chỗ: `api_key` còn trong `bean.toml`/`voice-state.json`; `~/.config/bean/env` còn tồn tại.
- Slot bị che: cùng tên ở env và `secrets.env` với hai giá trị khác nhau (so sánh băm, không in).
- Môi trường: `ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY` (K29), có nhiều tiến trình Bean hay không (K27),
  port bị chiếm.
- `llm-state.json` đang ghi đè `bean.toml` (hiển thị cả hai).
- Docker: user thuộc nhóm `docker`, image `bean-sandbox` tồn tại. Chrome: phát hiện bản **snap** và
  cảnh báo (hay lỗi CDP/sandbox trên Ubuntu).
- `--online` (opt-in): thử một request tối thiểu tới provider/voice/Telegram/search; lỗi được redact.

## 8. Web API và UI

### 8.1. REST (kiểu định nghĩa ở Rust, `ts-rs`, `make types`)

| Endpoint | Việc |
|---|---|
| `GET /api/settings` | Schema + giá trị hiệu lực (không-secret) với `{key, value, source, tier, apply, editable_in_ui}` |
| `PATCH /api/settings` | Sửa khoá tầng A. Khoá tầng B → `403 cli_only` kèm gợi ý lệnh |
| `GET /api/secrets` | `[{name, label, group, set, source, shadowed_by_env}]`. **Không có trường giá trị** |
| `PUT /api/secrets/:name` | `{value, password}` — step-up D19.5 |
| `DELETE /api/secrets/:name` | `{password}` |
| `POST /api/settings/test/:target` | `llm` \| `voice` \| `telegram` \| `search`: kiểm tra kết nối, lỗi đã redact |
| `POST /api/auth/change-password` | `{current, new}`; huỷ các phiên web khác |

Các endpoint cũ (`/api/config/llm/*`, `/api/voice`) **giữ nguyên hợp đồng**, nhưng nội bộ gọi
`SettingsService`; trường `api_key` trong body của chúng chuyển thành "ghi vào secret slot" và được
đánh dấu deprecated. Tất cả endpoint ghi: cookie + kiểm tra `Origin` + `Content-Type: application/json`
như mọi endpoint ghi khác; RBAC bật ⇒ chỉ `admin`; ghi audit.

### 8.2. UI (màn Cài đặt)

- Chia mục: Mô hình & provider · Giọng nói · Kênh (Telegram) · Tìm kiếm web · Tích hợp (billing/
  marketing) · Bảo mật & mật khẩu · Nâng cao (khoá tầng B, chỉ xem).
- **Ô secret write-only:** hiển thị "Đã đặt · nguồn: secrets.env" hoặc "Chưa đặt"; nút *Thay* /
  *Xoá*; ô nhập `type=password`, `autocomplete="new-password"`, **không bao giờ điền sẵn**; sau khi lưu
  xoá khỏi state và DOM. Nguồn `env` → khoá nút kèm giải thích ("đang bị biến môi trường ghi đè").
- Nút *Kiểm tra kết nối* cho từng nhóm. Banner "cần restart" khi có khoá `restart` đang chờ.
- Tuân thủ: một màn hình không cuộn trang (quy tắc 11), token màu (không màu Tailwind thô), `vi`/`en`,
  `aria`, không `any`, không `dangerouslySetInnerHTML`, chỉ dùng kiểu sinh bởi `ts-rs`.
- Không bao giờ lưu giá trị secret vào store zustand lâu dài, query cache của TanStack Query, hay
  `localStorage`.

## 9. Câu hỏi mở — agent phải nêu phương án và **hỏi trước khi làm**

1. **Huỷ phiên web khi đổi mật khẩu từ CLI.** DB chỉ qua một worker (D8.2, ràng buộc kiến trúc 2); CLI
   mở kết nối thứ hai khi server đang chạy là vấn đề. Xem cách `auth mcp-token` đang xử lý và đề xuất.
2. Chọn crate cho khoá file (`fd-lock` hoặc tương đương) và crate đọc mật khẩu ẩn: xác nhận còn
   bảo trì, đọc source (quy tắc 3).
3. Telegram đổi token: hot-restart kênh hay bắt buộc restart (mục 4.4).
4. Chỗ đặt `bean-settings` nếu có lý do tốt hơn mục 4.3.

## 10. Test bắt buộc

**Rust**

- `secrets.env` tạo quyền `0600` ngay lúc tạo; thư mục `0700`; ghi atomic (mô phỏng lỗi giữa chừng ⇒ file
  cũ còn nguyên).
- Tên ngoài slot (`PATH`, `LD_PRELOAD`, `HTTPS_PROXY`) bị từ chối; giá trị chứa xuống dòng/NUL bị từ chối.
- Thứ tự ưu tiên: env thắng file; `list` báo `shadowed_by_env`.
- Secret **không** xuất hiện trong: `Debug` của mọi struct cấu hình, log (`tracing` capture), audit,
  message lỗi, body `/api/*`, schema `ts-rs` sinh ra.
- Tiến trình con (MCP stdio, sandbox argv, scanner, Chrome) **không** nhận biến từ `secrets.env`.
- `read_file`/`grep`/`glob` với đường dẫn tới `secrets.env` bị từ chối; `workspace` chứa `data.dir` bị
  `validate()` từ chối.
- `config set`: khoá lạ bị từ chối; cấu hình không qua `validate()` thì **không ghi**; khoá tầng B cần TTY;
  comment trong `bean.toml` được giữ nguyên.
- `migrate-secrets`: dry-run không ghi gì; `--apply` chuyển đủ, tạo `*.pre-m29`, xoá khỏi file gốc,
  xung đột slot thì hỏi.
- Web: `GET /api/secrets` không chứa giá trị; `PUT` sai mật khẩu ⇒ 401 và tính vào giới hạn tần suất;
  `PATCH` khoá tầng B ⇒ 403 `cli_only`; sai `Origin` ⇒ 403; chưa đăng nhập ⇒ 401; RBAC bật + không phải
  admin ⇒ 403; mỗi thao tác ghi để lại audit không có giá trị.
- Không tool nào của agent đăng ký được đường ghi secret/cấu hình (test hồi quy toàn registry, theo mẫu
  `every_external_source_tool_declares_marks_untrusted`).

**Web (Vitest + MSW)**

- Ô secret không điền sẵn; sau khi lưu giá trị biến mất khỏi DOM; nguồn `env` khoá nút; khoá tầng B chỉ
  xem; lỗi 403 `cli_only` hiện gợi ý lệnh; đủ khoá i18n `vi`/`en`.

`make check`, `make audit`, `make e2e` xanh; `make types` không lệch.

## 11. Tài liệu phải cập nhật trong cùng lượt

- `docs/known-issues.md`: thêm **Q7** (đảo Q6, giữ nguyên văn Q6 kèm nhãn "đã bị thay thế bởi Q7");
  đóng **K30**; ghi chú vào K28; cập nhật K27 (beanctl thôi nạp env file).
- `docs/decisions.md`: mục mới **20 (D19.x)** ghi D19.1–D19.5 và lý do/đánh đổi của quyết định thực tế
  trong lúc làm.
- `AGENTS.md`: mục 0.6, 15.6, 18 (thêm các section/lệnh mới), 22.24.
- `README.md`: viết lại bảng "Trạng thái runtime và secret nằm ở đâu", mục cài đặt nhanh (`bean setup`),
  bảng sự cố; hướng dẫn nâng cấp (`migrate-secrets`).
- `Plan.md`: thêm M29 và trạng thái. `docs/status-report.md`: dòng M29.
- `bean.example.toml`: bỏ ví dụ `api_key`, thêm chú thích trỏ về `bean secret set`.

## 12. Ngoài phạm vi (ghi vào backlog, không làm)

- Mã hoá `secrets.env` tại chỗ, OS keyring, `systemd-creds`/`LoadCredential=` (cần đọc
  `$CREDENTIALS_DIRECTORY`; phù hợp cho kiểu triển khai systemd sau này).
- Gỡ hẳn trường `api_key` deprecated.
- Đa người dùng, quản lý secret theo role.
- Cho agent đọc/ghi thiết lập qua tool (**cấm** trong milestone này).

## 13. Định nghĩa xong

- Cài mới trên Ubuntu: `bean setup` đưa hệ thống tới trạng thái chạy được; chỉ có `secrets.env`
  (0600) chứa key, `bean.toml` không có key.
- Đặt/đổi key, đổi provider/model từ **UI** và từ **terminal** cho cùng kết quả, một nơi lưu.
- `bean doctor` sạch trên máy đã migrate; báo đúng khi cố tình đặt sai quyền hoặc để key sai chỗ.
- Mọi test ở mục 10 xanh. Commit `feat(m29): unified settings and secrets`. Báo cáo theo `AGENTS.md`
  mục 0.7, nêu rõ phương án đã chọn cho các câu hỏi mở ở mục 9.
