# BeanAgent

BeanAgent là personal AI agent self-hosted, viết bằng Rust. Runtime là **một binary Rust**;
React chỉ được build thành file tĩnh và nhúng trong binary khi phát hành. Node.js không cần
có trên máy chạy.

## Cài đặt nhanh

Yêu cầu build: Rust toolchain trong `rust-toolchain.toml`, Node LTS + pnpm chỉ trong bước build web.
Yêu cầu runtime: Linux, một thư mục dữ liệu riêng và Docker nếu dùng sandbox mặc định.

```bash
cp BeanAgent.example.toml BeanAgent.toml
cp BeanAgent.example.toml /tmp/BeanAgent.toml.example
# sửa [llm], [web], [telegram], [security] và đặt secret trong biến môi trường
export ANTHROPIC_API_KEY='...'
make check
make build
./target/release/BeanAgent auth set-password
./target/release/BeanAgent serve
```

Mở `http://127.0.0.1:7878`. `auth set-password` cần TTY và chỉ lưu hash Argon2id trong
`data.dir/auth.toml` với quyền `0600`; web không bật được nếu chưa có file này. Không ghi API key,
bot token hoặc password vào `BeanAgent.toml`.

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

File mẫu đầy đủ là `BeanAgent.example.toml`. Các trường quan trọng:

- `[agent]`: `workspace`, `max_steps`, `context_budget_tokens`, `timezone`, `allowed_users`.
- `[security].daily_token_budget`: ngân sách token theo ngày UTC; khi chạm/vượt, run dừng và
  ghi thông báo bền vững để UI, Telegram và lịch sử sau reconnect đều thấy.
- `[security.sandbox]`: `docker` hoặc `host`; `host` làm mọi `run_shell` thành Dangerous.
- `[web]`: mặc định bind loopback, `public_origin`, `allow_remote`, `trust_proxy`.
- `[telegram]`: `enabled`, `token_env`, `allowed_user_ids`; user ID cũng phải có trong
  `agent.allowed_users` dưới dạng `telegram:<id>`.
- `[[mcp_servers]]`: stdio command/args/env; `trust = false` làm mỗi MCP tool cần xác nhận.

### Web an toàn khi truy cập từ xa

Mặc định chỉ nghe `127.0.0.1`. Không bind trực tiếp ra Internet. Một trong hai cách an toàn:

1. Đặt sau reverse proxy Caddy/nginx có TLS, giữ BeanAgent bind loopback và đặt
   `public_origin = "https://agent.example.com"`; `trust_proxy = true` chỉ khi proxy đã được
   kiểm soát và luôn chỉ truyền `X-Forwarded-For` từ proxy đó.
2. Dùng Tailscale/WireGuard/VPN riêng, truy cập bằng địa chỉ private và đặt
   `public_origin` khớp chính xác với URL đó.

`allow_remote = true` chỉ là cờ ý thức rủi ro; BeanAgent không tự làm TLS. Cookie có
`HttpOnly`, `SameSite=Strict`, `Secure` khi origin HTTPS; mọi request thay đổi dữ liệu kiểm tra
Origin/Host và `Content-Type: application/json`.

## Telegram

1. Nhắn `@BotFather`, tạo bot và lấy token.
2. Đặt token trong biến môi trường được trỏ bởi `telegram.token_env` (ví dụ
   `TELEGRAM_BOT_TOKEN`), không ghi vào file cấu hình.
3. Lấy user ID của bot/user và thêm vào `telegram.allowed_user_ids` và
   `agent.allowed_users`.
4. Bật `[telegram] enabled = true`, chạy `BeanAgent serve`.

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
Nếu server transport rớt, BeanAgent thử reconnect một lần với backoff; lỗi vẫn được trả về
model dưới dạng tool error. Có thể bọc server không tin cậy bằng Docker, nhưng không mount
Docker socket chỉ để chạy MCP.


## Docker

Build image nhiều tầng (Node chỉ ở stage build; runtime là Debian slim, không có Node):

```bash
docker build -f Dockerfile -t beanagent:local .
docker build -f Dockerfile.sandbox -t beanagent-sandbox:local .
```

Chạy agent, mount cấu hình/data/workspace và chỉ bind loopback:

```bash
docker run --rm --name beanagent \
  -p 127.0.0.1:7878:7878 \
  -v "$PWD/BeanAgent.toml:/etc/beanagent/BeanAgent.toml:ro" \
  -v "$HOME/.BeanAgent:/var/lib/beanagent" \
  -v "$PWD/workspace:/srv/beanagent/workspace" \
  --env-file /secure/path/beanagent.env \
  beanagent:local
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
sudo useradd --system --home /var/lib/beanagent --create-home --shell /usr/sbin/nologin beanagent
sudo install -d -o beanagent -g beanagent -m 0700 /srv/beanagent/workspace
sudo install -o root -g beanagent -m 0640 target/release/BeanAgent /usr/local/bin/BeanAgent
sudo install -d -o root -g beanagent -m 0750 /etc/beanagent
sudo install -o root -g beanagent -m 0640 BeanAgent.toml /etc/beanagent/BeanAgent.toml
sudo install -o root -g beanagent -m 0640 deploy/systemd/BeanAgent.service /etc/systemd/system/BeanAgent.service
sudo install -o root -g root -m 0600 /secure/path/beanagent.env /etc/beanagent/beanagent.env
sudo -u beanagent BeanAgent --config /etc/beanagent/BeanAgent.toml auth set-password
sudo systemctl daemon-reload
sudo systemctl enable --now BeanAgent
```

File unit dùng `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`, `PrivateDevices`,
giới hạn namespace/control groups và chỉ cho phép ghi vào data/workspace. Nó cần thêm user
`beanagent` vào nhóm `docker` nếu dùng Docker sandbox. **Thành viên nhóm `docker` gần như
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
- Web đã có auth, CSRF/Origin, CSP, rate limit; BeanAgent không cung cấp TLS, quota
  nhiều người dùng, sandbox kernel-level hoặc bảo mật tương đương VM.
- Một token bot Telegram chỉ nên chạy một instance. `allowed_users` là lớp phòng thủ thứ hai
  sau allowlist của channel; không thêm user chỉ để “thử”.
- MCP executable là code không tin cậy. `trust = true` chỉ dành cho server đã audit; luôn
  kiểm tra command, args, env và quyền mount.
- `max_steps` và `daily_token_budget` chỉ là giới hạn runtime, không phải quota tài chính
  tuyệt đối nếu provider đã trả response vượt ngưỡng; BeanAgent ghi nhận usage thực trả về
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

Sau `make build`, chạy `target/release/BeanAgent --help` hoặc E2E với `PATH` đã loại Node để
xác nhận binary tự chứa giao diện và không phụ thuộc runtime Node. `make check` là cổng
chất lượng bắt buộc trước khi commit M16.
