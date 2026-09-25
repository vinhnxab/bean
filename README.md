# BeanAgent

Personal AI agent self-hosted viết bằng Rust. Xem `AGENTS.md` cho kiến trúc và phạm vi từng milestone.

## Kiểm tra và build

```bash
make check          # fmt, clippy, test Rust, type export, test web
make build-headless # binary Rust không cần Node
make build          # build web rồi nhúng UI vào binary release
```

## Learning loop (M15)

Sau một run thành công có đủ số tool call, BeanAgent gọi LLM reflection và chỉ tạo đề xuất
trong `skills/_drafts/<name>/`; draft không được loader sử dụng cho tới khi người dùng duyệt.

```toml
[learning]
enabled = true
min_tool_calls = 5
proposal_interval_minutes = 60
```

Duyệt hoặc bỏ bằng `/approve <id>` / `/reject <id>`, nút inline Telegram, hoặc màn Skills
trong web. Skill mới chỉ được kích hoạt sau khi duyệt; bản sửa chỉ nhận skill đã được load
trong chính run đó. Timestamp đề xuất được lưu cạnh draft nên giới hạn tần suất vẫn giữ
qua restart.

## MCP client (M14)

BeanAgent dùng SDK Rust chính thức `rmcp` để nạp MCP server qua stdio. Khai báo một hoặc
nhiều server trong `BeanAgent.toml`:

```toml
[[mcp_servers]]
name = "docs" # chỉ chữ, số, `_`, `-`; thành tiền tố tool
command = "/usr/local/bin/my-mcp-server"
args = ["--stdio"]
# env = { APP_MODE = "production" }
trust = false
```

Tool `search` của server trên được đăng ký thành `mcp__docs__search`. Mặc định
`trust = false` nên mỗi lần gọi đều cần xác nhận; chỉ đặt `trust = true` sau khi đã kiểm
định server. `command` là executable (không phải một dòng shell), còn `args` là danh sách
đối số. `env` chỉ truyền các biến liệt kê cho tiến trình con; host environment không được
kế thừa. Không ghi secret vào `BeanAgent.toml`.

### Bọc MCP server trong container

`command` có thể là `docker`; `args` truyền nguyên vẹn lệnh `docker run`. Giữ stdio bằng
`--interactive` và **không dùng `--tty`**:

```toml
[[mcp_servers]]
name = "isolated"
command = "docker"
args = [
  "run", "--rm", "--interactive",
  "--network", "none",
  "--read-only",
  "--cap-drop", "ALL",
  "--security-opt", "no-new-privileges",
  "--pids-limit", "64",
  "--memory", "256m",
  "--cpus", "1",
  "--user", "65532:65532",
  "--tmpfs", "/tmp:rw,noexec,nosuid,size=64m",
  "ghcr.io/example/my-mcp-server:1.0.0",
]
trust = false
```

Không mount Docker socket hoặc workspace vào container MCP chỉ để chạy server. Nếu server
cần workspace, hãy đặt chính sách mount tối thiểu và hiểu rõ server sẽ thấy dữ liệu đó.
BeanAgent không shell-parse `command`, nên cũng có thể trỏ `command` tới một wrapper script
đã kiểm soát.

Lỗi spawn, initialize, discovery, timeout hoặc shutdown của một server chỉ được log cảnh
báo; server còn lại và agent vẫn khởi động. Kết quả MCP luôn được bọc trong
`<untrusted_content>…</untrusted_content>` và đánh dấu lượt hiện tại là đã đọc dữ liệu
không tin cậy.

### Test server Rust trong repository

`crates/beanagent-mcp-test-server` là stdio server tối thiểu, dùng để kiểm tra handshake,
discovery, tool call, timeout, untrusted output và child-process cleanup mà không cần Node:

```bash
cargo test -p beanagent-tools -p beanagent-mcp-test-server
```
