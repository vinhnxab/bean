# Bean

Bean là personal AI agent self-hosted, viết bằng Rust. Runtime là **một binary Rust**;
React chỉ được build thành file tĩnh và nhúng trong binary khi phát hành. Node.js không cần
có trên máy chạy.

## Cài đặt nhanh

Yêu cầu build: Rust toolchain trong `rust-toolchain.toml`, Node LTS + pnpm chỉ trong bước build web.
Yêu cầu runtime: Linux, một thư mục dữ liệu riêng và Docker nếu dùng sandbox mặc định.

**Bộ nhớ khi build: cần tối thiểu ~4 GB RAM khả dụng** nếu bật tool browser. Số đo
trên máy dev (8 nhân): crate `chromiumoxide_cdp` sinh **111.222 dòng** kiểu CDP tự
động và một mình nó chiếm **165,5 s** biên dịch (`chromiumoxide_pdl` 9,6 s,
`chromiumoxide` 41,6 s); peak RSS của `cargo build` đo được **2,64 GB** — không tính
phần build song song các crate khác. Nếu không dùng tool browser thì có thể xoá dòng
`chromiumoxide` khỏi `[workspace.dependencies]` và `bean-browser` khỏi
`[workspace].members` trước khi build.

```bash
cp bean.example.toml bean.toml
cp bean.example.toml /tmp/bean.toml.example
# sửa [llm], [web], [telegram], [security] và đặt secret trong biến môi trường
export ANTHROPIC_API_KEY='...'
make check
make build
./target/release/bean auth set-password
./target/release/bean serve
```

Mở `http://127.0.0.1:7878`. `auth set-password` cần TTY và chỉ lưu hash Argon2id trong
`data.dir/auth.toml` với quyền `0600`; web không bật được nếu chưa có file này. Không ghi API key,
bot token hoặc password vào `bean.toml`.

> **Nâng cấp từ bản cũ (đổi tên 2026-09-28).** Tên sản phẩm đổi `BeanAgent` → `bean`, nên
> các đường dẫn mặc định cũng đổi theo và **không có migration tự động**: chạy
> `mv ~/.BeanAgent ~/.bean` một lần (giữ nguyên `auth.toml`, `beanagent.db`→`bean.db` nằm
> trong đó), và đổi tên biến môi trường `BEANAGENT_*` → `BEAN_*` trong shell/service.
> Unit systemd đổi tên nên phải `systemctl disable beanagent` trước khi cài
> `deploy/systemd/bean.service`. Chi tiết: `docs/decisions.md` mục 19 (D18.3, D18.5).

## Build và kiểm tra

```bash
make check             # fmt, clippy, test Rust, type export, Biome, TS, Vitest, web build
make build             # web build -> cargo release --features ui (binary tự chứa UI)
make build-headless    # Rust release không UI, chạy được khi máy không có Node
make audit             # cargo audit/deny + pnpm audit --prod
make e2e               # binary serve --fake-llm: login, chat, confirm, Stop, reconnect Sync
make smoke-scheduler   # smoke 1 giờ ảo, scheduler tick nhanh
```

`cargo audit` chạy strict (`-D warnings`). M16 ghi ngoại lệ tường minh
`RUSTSEC-2026-0173`: đây là cảnh báo unmaintained của `aquamarine` chỉ đi qua `teloxide 0.17`
(latest hiện tại), không phải vulnerability; không có bản sửa thay thế. Lock file Rust và
`web/pnpm-lock.yaml` phải được commit.

## Cấu hình

File mẫu đầy đủ là `bean.example.toml`. Các trường quan trọng:

- `[agent]`: `workspace`, `max_steps`, `context_budget_tokens`, `timezone`, `allowed_users`.
- `[llm]`: `provider` (`anthropic` | `openai_compat`), `model`, `allowed_models`, `api_key_env`,
  `base_url`, `max_tokens`. `api_key_env` là **tên biến môi trường**, không phải key.
- `[security].daily_token_budget`: ngân sách token theo ngày UTC; khi chạm/vượt, run dừng và
  ghi thông báo bền vững để UI, Telegram và lịch sử sau reconnect đều thấy.
- `[security.sandbox]`: `docker` hoặc `host`; `host` làm mọi `run_shell` thành Dangerous.
- `[web]`: mặc định bind loopback, `public_origin`, `allow_remote`, `trust_proxy`.
- `[telegram]`: `enabled`, `token_env`, `allowed_user_ids`; user ID cũng phải có trong
  `agent.allowed_users` dưới dạng `telegram:<id>`.
- `[[mcp_servers]]`: stdio command/args/env; `trust = false` làm mỗi MCP tool cần xác nhận.
- `[mcp_server]` + `[[mcp_clients]]`: biến Bean thành **MCP server read-only** cho Cline/Cursor/
  OpenCode/Claude Code (xem phần kế tiếp). Mặc định tắt.

### Dùng Bean như MCP server (read-only)

Agent khác gọi vào Bean để đọc dữ liệu Bean đang quản lý: log hạ tầng, chi phí cloud, ghi
chú `MEMORY.md`/`USER.md`. **Không** có đường nào để ghi file hay chạy lệnh — cổng expose
lọc theo tag + `Risk::Safe` ở tầng code, nên kể cả client gắn role `admin` cũng không gọi
được tool ghi.

```toml
[mcp_server]
enabled = true

[[roles]]
name = "monitor"
tool_tags = ["infra-read", "memory-read"]

[agent]
# Bắt buộc: nếu thiếu dòng này thì client là `no-access` và không thấy tool nào
# (Bean sẽ báo lỗi ngay lúc nạp cấu hình).
user_roles = { "mcp-client:cline" = "monitor" }

[[mcp_clients]]
name = "cline"
role = "monitor"
```

```sh
bean auth mcp-token add cline      # in token MỘT LẦN duy nhất
bean auth mcp-token list          # xem client đã cấp
bean auth mcp-token revoke cline  # thu hồi
```

**stdio (khuyến nghị, cùng máy với IDE)** — trong Cline/Cursor trỏ:

```json
{ "command": "bean", "args": ["mcp", "serve"],
  "env": { "BEAN_MCP_TOKEN": "<token vừa sinh>" } }
```

**HTTP (Bean chạy từ xa)** — `bean mcp serve --http`, client POST tới
`{public_origin}/mcp` với header `Authorization: Bearer <token>`. Mặc định chỉ bind
loopback; muốn lộ ra ngoài phải đặt `allow_remote = true` **và** đặt sau reverse proxy
TLS/Tailscale (giống `[web]`).

Token là bí mật dài hạn nằm trong thư mục dự án — **đừng commit** nó. Bean chỉ lưu
SHA-256 trong `data.dir/bean.db`; thu hồi bằng `auth mcp-token revoke`.

### OpenRouter (và mọi endpoint tương thích OpenAI)

OpenRouter nói đúng dialect OpenAI Chat Completions nên chỉ cần `provider = "openai_compat"`
và `base_url`. Đặt key trong biến môi trường, **không** ghi vào file cấu hình:

```sh
export OPENROUTER_API_KEY='sk-or-v1-...'   # thêm vào ~/.bashrc, không commit
```

```toml
[llm]
provider = "openai_compat"
model = "nvidia/nemotron-3-super-120b-a12b:free"
allowed_models = ["nvidia/nemotron-3-super-120b-a12b:free", "anthropic/claude-sonnet-5"]
api_key_env = "OPENROUTER_API_KEY"          # tên biến, không phải key
base_url = "https://openrouter.ai/api/v1"
max_tokens = 2048
```

```sh
./target/debug/bean chat
```

Xem model và giá hiện tại: `curl -s https://openrouter.ai/api/v1/models | jq -r '.data[].id'`.
Xem credit/giới hạn của key: `curl -s -H "Authorization: Bearer $OPENROUTER_API_KEY" \
https://openrouter.ai/api/v1/key`.

Hai lỗi thường gặp:

- **HTTP 402 "requires more credits, or fewer max_tokens"** — tài khoản free tier, credit ≈ 0.
  Dùng model hậu tố `:free` (giới hạn request/ngày) hoặc nạp credit.
- **HTTP 404 "0 endpoints ... guardrail restrictions and data policy"** — endpoint bị loại do
  cài đặt **ZDR / data policy** trong tài khoản. Sửa ở
  <https://openrouter.ai/settings/privacy>, hoặc chọn model khác.

### Web an toàn khi truy cập từ xa

Mặc định chỉ nghe `127.0.0.1`. Không bind trực tiếp ra Internet. Một trong hai cách an toàn:

1. Đặt sau reverse proxy Caddy/nginx có TLS, giữ Bean bind loopback và đặt
   `public_origin = "https://agent.example.com"`; `trust_proxy = true` chỉ khi proxy đã được
   kiểm soát và luôn chỉ truyền `X-Forwarded-For` từ proxy đó.
2. Dùng Tailscale/WireGuard/VPN riêng, truy cập bằng địa chỉ private và đặt
   `public_origin` khớp chính xác với URL đó.

`allow_remote = true` chỉ là cờ ý thức rủi ro; Bean không tự làm TLS. Cookie có
`HttpOnly`, `SameSite=Strict`, `Secure` khi origin HTTPS; mọi request thay đổi dữ liệu kiểm tra
Origin/Host và `Content-Type: application/json`.

## Telegram

1. Nhắn `@BotFather`, tạo bot và lấy token.
2. Đặt token trong biến môi trường được trỏ bởi `telegram.token_env` (ví dụ
   `TELEGRAM_BOT_TOKEN`), không ghi vào file cấu hình.
3. Lấy user ID của bot/user và thêm vào `telegram.allowed_user_ids` và
   `agent.allowed_users`.
4. Bật `[telegram] enabled = true`, chạy `bean serve`.

Telegram long polling có reconnect/backoff, xử lý lỗi 409 rõ ràng, giới hạn tần suất và chỉ
nhận callback của đúng user trong allowlist. Tin cần xác nhận dùng inline keyboard; Dangerous
không có nút cho phép trong phiên. Một instance phải dùng một bot token duy nhất.

## Skills và MCP

Skill nằm trong `skills/<ten-kebab-case>/SKILL.md` hoặc `data.dir/skills/<ten>/SKILL.md`, có
frontmatter `name` và `description`; loader chỉ đưa index vào system prompt, model gọi
`load_skill` để nạp nội dung đầy đủ. Skill nháp chỉ hoạt động sau khi duyệt.

MCP server là executable stdio, không phải một dòng shell:

```toml
[[mcp_servers]]
name = "docs"
command = "/usr/local/bin/docs-mcp"
args = ["--stdio"]
env = { APP_MODE = "production" }
trust = false
```

Tool được đặt tên `mcp__docs__<tool>`. `trust = false` mặc định cần Confirm; chỉ đặt `true`
sau khi đã kiểm tra server. `env` chỉ truyền biến liên kết, không kế thừa secret của host.
Nếu server transport rớt, Bean thử reconnect một lần với backoff; lỗi vẫn được trả về
model dưới dạng tool error. Có thể bọc server không tin cậy bằng Docker, nhưng không mount
Docker socket chỉ để chạy MCP.

Hai trường cho server cần môi trường của tiến trình Bean hoặc chạy lâu:

- `inherit_env = ["PATH", "HOME"]` — Bean spawn MCP server với `env_clear()` để server
  không thấy secret của host, nhưng `env_clear()` cũng xoá `PATH` và server cần `PATH` để
  chạy sẽ chết ngay với status 127 (wrapper có shebang `#!/usr/bin/env <interpreter>`).
  Trường này là danh sách trắng bật lại đúng những biến cần; `env` tường minh luôn thắng.
  Tên biến chứa `key`/`token`/`secret`/`password` sẽ log cảnh báo.
- `call_timeout_seconds = 300` — timeout cho một lần gọi tool (mặc định 60s). Cần cho
  tool chạy lâu như truy vấn SIEM hay xuất báo cáo.

MCP server phải được cài sẵn trên máy: Bean chỉ spawn executable, không tải gì lúc chạy.

## Tool browser (Chrome DevTools Protocol)

Bean nói thẳng **Chrome DevTools Protocol** bằng crate Rust `chromiumoxide` — không qua
MCP, không qua npm, không cần Node ở bất kỳ đâu. Bật bằng `[browser]` **và** thêm
`"browser"` vào `[tools] enabled`:

```toml
[browser]
enabled = true
# Rỗng => tự phát hiện Chrome đã cài: biến CHROME, rồi `which google-chrome`
# / `chromium`, rồi /opt/google/chrome. Bean KHÔNG tải Chrome lúc chạy.
executable_path = ""
# Origin được phép chạy tool hành động ở mức Confirm (có "cho phép trong phiên").
allowed_origins = ["http://localhost:3000", "https://*.dev.internal"]
headless = true
idle_timeout_seconds = 300   # idle quá hạn thì tự tắt Chrome
```

**Hai nhóm tool, phân quyền theo RBAC sẵn có:**

| Nhóm | Tool | Tag | Mức rủi ro |
|---|---|---|---|
| Đọc | `browser_screenshot`, `browser_console_logs`, `browser_network`, `browser_performance_trace` | `dev-read` **hoặc** `test-run` | `Safe` |
| Hành động | `browser_navigate`, `browser_click`, `browser_fill`, `browser_press_key` | `test-run` (**chỉ QA**) | `Confirm` nếu origin trong whitelist, `Dangerous` nếu ngoài |
| Hành động | `browser_evaluate_script` | `test-run` | **luôn** `Dangerous` |

Role `developer` **không thấy và không gọi được** bất kỳ tool hành động nào — nguyên tắc
four-eyes y hệt `run_shell`/`write_file`. Mọi tool trong nhóm này bọc kết quả trong
`<untrusted_content>` và bật cờ untrusted, nên sau khi đọc trang, mọi hành động `Confirm`
trở lên trong lượt đó đều hỏi lại (mất tuỳ chọn "cho phép trong phiên").

**`allowed_origins` khớp theo ranh giới label, không theo hậu tố chuỗi.** `*.dev.internal`
khớp `api.dev.internal` nhưng **không** khớp `dev.internal.attacker.com` — domain đó kết
thúc bằng chuỗi giống hệt và thuộc quyền kiểm soát của kẻ tấn công. Scheme và port phải
khớp tuyệt đối (không bao giờ hạ `https` xuống `http`; không ghi port nghĩa là chỉ port mặc
định của scheme). **Danh sách rỗng không có nghĩa "cho phép tất cả"** — mọi origin ngoài
whitelist vẫn là `Dangerous`.

**Ngoại lệ loopback.** `http://localhost:3000` là môi trường test nên vẫn phải mở được,
dù lớp chống SSRF của toàn hệ thống chặn loopback. Vì vậy tool browser dùng **lớp kiểm
riêng** và chỉ mở loopback **khi origin nằm trong `allowed_origins`**; mọi dải nội bộ khác
(`10/8`, `172.16/12`, `192.168/16`, `169.254.169.254`, `fc00::/7`…) vẫn bị chặn cứng.
Xem `docs/known-issues.md` về khoảng hở DNS rebinding còn sót lại.

**Vòng đời tiến trình Chrome.** Một tiến trình dùng chung cho cả phiên Bean (khởi động
mất 1–3 giây, và chết giữa chuỗi lệnh sẽ mất sạch cookie/localStorage của phiên test).
Dọn tiến trình con qua 4 lớp: tắt khi Bean dừng; tự tắt khi idle; dọn tiến trình mồ côi
của lần chạy trước (đọc `DevToolsActivePort`, chỉ kill khi `/proc/<pid>/cmdline` chứa đúng
`--user-data-dir` của Bean nên không nhầm PID); `Browser::launch` tự kill con khi lỗi.
Profile nằm ở `data.dir/browser/profile` — cô lập, **không bao giờ** dùng
`~/.config/google-chrome` của bạn.

**Ảnh.** `browser_screenshot` là tool **duy nhất** trả ảnh; nó dùng kiểu nội dung riêng
mà **không** phải đổi chữ ký của 25+ tool cũ. Ảnh có định phí token **cố định**
(`IMAGE_BUDGET_TOKENS`), không dùng công thức `chars/4` — tính theo ký tự, một ảnh 200 KB
sẽ thành ~67.500 token và tự loại sạch lịch sử của lượt đó. `audit.jsonl` **không** bao
giờ ghi base64, chỉ ghi tham chiếu `image:<media_type>:<sha256>:<độ dài>`.


## QA test-runner (vai trò `qa` chạy test có sẵn)

Tag `test-run` đã có từ M21 nhưng trước M27 **chưa tool nào dùng**, nên vai trò `qa` không
chạy được lệnh test nào. `qa_test` đóng đúng khoảng trống đó: chạy **test suite có sẵn**
của dự án trong container với workspace mount **read-only**.

```toml
[qa]
enabled = true

[qa.sandbox]
image = "bean-sandbox:latest"   # image PHẢI chứa toolchain của runner
timeout_seconds = 900                 # test lâu hơn shell nhiều

[[qa.suites]]
name = "core-unit"                    # model chỉ được chọn đúng tên này
runner = "cargo_test"                 # cargo_test | vitest | pytest
workdir = ""                          # tương đối workspace; rỗng = gốc
args = ["--workspace"]                # tham số CỐ ĐỊNH, code tự dựng argv
```

**Ba ranh giới được ràng buộc ở tầng code, không phải lời hứa cho model:**

1. **Chỉ chạy suite đã khai báo.** Tool không có tham số `command`/`argv`/`workdir`;
   model chỉ chọn `suite_name` + `filter`, mọi argv do code dựng. `[[qa.suites]]` rỗng
   ⇒ **mọi** lần gọi bị từ chối (fail-closed, y hệt `[[infra_scope]]` rỗng của M23).
2. **Không gì được ghi vào dự án.** Workspace mount `:ro` + mọi thư mục cache
   (`CARGO_TARGET_DIR`, `CARGO_HOME`, `HOME`…) trỏ ra `/tmp` của container. Kể cả
   `build.rs` hay test tự cố ghi cũng **không** chạm được vào cây thư mục dự án.
   `QaSandboxConfig` cố ý **không có** trường `mode` — chỉ chạy được trong container.
3. **Báo cáo chuẩn hoá, không phải raw log.** Manager đọc `{status, summary, risks,
   failed_tests}` giống hệt `ScanReport` của M23. Cả report lẫn raw log đều bọc
   `<untrusted_content>` vì tên test/fixture/assertion do code dự án kiểm soát.

**Về four-eyes.** Vai trò `qa` chạy được `qa_test` nhưng **không** thấy/gọi được
`write_file`, `edit_file`, `run_shell` — `forbid_tags = ["dev-write"]` giữ nguyên, và
M27 không mở bất kỳ đường ghi nào (kể cả thư mục riêng của QA).

**Mức rủi ro `Confirm`, và CÓ tuỳ chọn "cho phép trong phiên"** — khác `security_scan` của
M23 (`Dangerous`, không có tuỳ chọn). Lý do: `qa_test` chỉ đọc workspace read-only nên
không đụng hệ thống ngoài, còn một phiên review thực tế lặp "test hỏng → sửa → chạy lại"
nhiều lần. **Điều kiện ràng buộc**: mount `:ro` phải giữ nguyên; nới thành ghi được thì
phải xét lại quyết định này. Xem `docs/decisions.md` D17.1–D17.8.

**Cờ mạng không cấu hình được.** `cargo_test` luôn có mạng (`CARGO_HOME=/tmp` trong
container `--rm` nên không có cache crates.io để tải lại), `vitest`/`pytest` luôn không
mạng. Khai khác thì `validate()` ghi đè và in cảnh báo — xem D17.2.

## Docker

Build image nhiều tầng (Node chỉ ở stage build; runtime là Debian slim, không có Node):

```bash
docker build -f Dockerfile -t bean:local .
docker build -f Dockerfile.sandbox -t bean-sandbox:local .
```

Chạy agent, mount cấu hình/data/workspace và chỉ bind loopback:

```bash
docker run --rm --name bean \
  -p 127.0.0.1:7878:7878 \
  -v "$PWD/bean.toml:/etc/bean/bean.toml:ro" \
  -v "$HOME/.bean:/var/lib/bean" \
  -v "$PWD/workspace:/srv/bean/workspace" \
  --env-file /secure/path/bean.env \
  bean:local
```

Nếu chạy agent trong container và cần sandbox Docker, phải mount socket hoặc cấu hình
Docker client. **Mount `/var/run/docker.sock` gần như tương đương root trên host**: bất kỳ
process nào có quyền ghi socket đều có thể chiếm quyền root, vượt qua isolation của agent.
Nếu không chấp nhận rủi ro này, dùng rootless Docker/Podman, Podman rootless với
`podman.socket` hạn chế, hoặc chạy agent host-side với sandbox rootless. Không mount socket
vào image sandbox tool.

## Cài systemd

Binary release không cần Node. Cài user thật, file cấu hình và workspace:

```bash
sudo useradd --system --home /var/lib/bean --create-home --shell /usr/sbin/nologin bean
sudo install -d -o bean -g bean -m 0700 /srv/bean/workspace
sudo install -o root -g bean -m 0640 target/release/bean /usr/local/bin/bean
sudo install -d -o root -g bean -m 0750 /etc/bean
sudo install -o root -g bean -m 0640 bean.toml /etc/bean/bean.toml
sudo install -o root -g bean -m 0640 deploy/systemd/bean.service /etc/systemd/system/bean.service
sudo install -o root -g root -m 0600 /secure/path/bean.env /etc/bean/bean.env
sudo -u bean bean --config /etc/bean/bean.toml auth set-password
sudo systemctl daemon-reload
sudo systemctl enable --now bean
```

File unit dùng `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`, `PrivateDevices`,
giới hạn namespace/control groups và chỉ cho phép ghi vào data/workspace. Nó cần thêm user
`bean` vào nhóm `docker` nếu dùng Docker sandbox. **Thành viên nhóm `docker` gần như
tương đương root**, vì Docker daemon chạy với quyền root. Đây là quyết định trust boundary
cần hiểu rõ; phương án an toàn hơn là rootless Docker/Podman hoặc bỏ supplementary group
và dùng sandbox rootless. `EnvironmentFile` không được đưa vào Git.

## Logging, backup và vận hành

Log chạy ở chế độ JSON qua `tracing`, điều khiển bằng `RUST_LOG`; secret trong log/audit được
redact, còn auth token chỉ lưu hash. `SIGTERM` và `SIGINT` kích hoạt shutdown êm: huỷ
Router runs, dừng scheduler, channel, WebSocket server và đóng MCP. Telegram/MCP có
reconnect; WebSocket client phải gửi `Sync` lại và nạp REST history.

Backup cả `data.dir` (SQLite, auth, audit) và `workspace`; không backup riêng `auth.toml` mà
bỏ qua database nếu cần khôi phục phiên đăng nhập. Cấu hình và secret có thể tách khỏi
backup để giảm bề mặt lộ dữ liệu.

## Mô hình đe doạ và giới hạn

- Agent có thể đọc/ghi workspace và gọi tool; file bị capability jail, nhưng host shell
  (`sandbox.mode = "host"`) hoặc Docker socket là quyền tương đương root.
- Nội dung web/file/email/MCP là untrusted và có thể chứa prompt injection; system prompt và
  UI không được xem nội dung đó là chỉ dẫn đáng tin. Mọi tool đọc nội dung từ nguồn ngoài lõi
  (`web_fetch`, `web_search`, `read_file`, `grep`, `glob`, `list_dir`, output `run_shell`, MCP)
  đều bọc `<untrusted_content>`; sau khi đọc trong một lượt, mọi tool Confirm/Dangerous bắt
  buộc hỏi lại và mất tuỳ chọn "cho phép trong phiên".
- `web_fetch` chặn SSRF cơ bản, nhưng proxy/DNS riêng và endpoint nội bộ cần review thêm.
- Web đã có auth, CSRF/Origin, CSP, rate limit; Bean không cung cấp TLS, quota
  nhiều người dùng, sandbox kernel-level hoặc bảo mật tương đương VM.
- Một token bot Telegram chỉ nên chạy một instance. `allowed_users` là lớp phòng thủ thứ hai
  sau allowlist của channel; không thêm user chỉ để “thử”.
- MCP executable là code không tin cậy. `trust = true` chỉ dành cho server đã audit; luôn
  kiểm tra command, args, env và quyền mount.
- `max_steps` và `daily_token_budget` chỉ là giới hạn runtime, không phải quota tài chính
  tuyệt đối nếu provider đã trả response vượt ngưỡng; Bean ghi nhận usage thực trả về
  và dừng run ngay sau đó.
- Telegram thật, provider thật và browser Playwright cần credential/môi trường riêng; M16
  E2E dùng fake provider và adapter test, không ghi secret vào CI. `make smoke-scheduler`
  mô phỏng một giờ với clock/tick nhanh để kiểm tra logic, không thay thế uptime 24 giờ.

## Kiểm tra trước khi phát hành

```bash
make check
make audit
make e2e
make build
make build-headless
```

Sau `make build`, chạy `target/release/bean --help` hoặc E2E với `PATH` đã loại Node để
xác nhận binary tự chứa giao diện và không phụ thuộc runtime Node. `make check` là cổng
chất lượng bắt buộc trước khi commit M16.
