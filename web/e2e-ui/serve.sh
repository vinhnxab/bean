#!/usr/bin/env bash
# Bootstrap `bean serve --fake-llm` cho Playwright E2E UI (mục 3, M16).
#
# An toàn tuyệt đối với dữ liệu thật:
#   - Toàn bộ data.dir + workspace nằm trong thư mục temp riêng của lần chạy.
#   - HOME bị ép về temp: mọi chỗ mở rộng `~` (expand_tilde) cũng rơi vào temp,
#     không bao giờ chạm ~/.bean thật — bài học từ lần chạy thử nhầm auth.toml.
#   - Port mặc định 18944, khác 7878 của instance thật (chạy bằng beanctl).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/debug/bean"
TMP="$(mktemp -d /tmp/bean-e2e-ui.XXXXXX)"
PORT="${BEAN_E2E_PORT:-18944}"
PASSWORD="e2e-password-123456"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

mkdir -p "$TMP/home" "$TMP/data" "$TMP/workspace"
export HOME="$TMP/home"

if [[ ! -x "$BIN" ]]; then
  (cd "$ROOT" && cargo build -p bean --locked)
fi

cat > "$TMP/bean.toml" <<EOF
[agent]
workspace = "$TMP/workspace"
allowed_users = ["web:admin"]

[data]
dir = "$TMP/data"

[tools]
enabled = ["files", "memory", "skills"]

[web]
enabled = true
bind = "127.0.0.1:$PORT"
public_origin = "http://127.0.0.1:$PORT"

[telegram]
enabled = false
EOF

# `bean auth set-password` đòi TTY (nhập ẩn) — bọc bằng pty giả của util-linux,
# bơm mật khẩu hai lần (mật khẩu + xác nhận).
printf '%s\n%s\n' "$PASSWORD" "$PASSWORD" \
  | script -qec "\"$BIN\" --config \"$TMP/bean.toml\" auth set-password" /dev/null >/dev/null

"$BIN" --config "$TMP/bean.toml" serve --fake-llm "$ROOT/web/e2e-ui/fake-script.json" &
SERVER_PID=$!
wait "$SERVER_PID"
