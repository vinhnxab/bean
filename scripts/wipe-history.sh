#!/bin/sh
# Xoá toàn bộ lịch sử trò chuyện (sessions + messages) khỏi SQLite của Bean.
# Giữ nguyên: memories, audit log, skills, tasks, web_sessions (không phải đăng nhập lại).
#
# Cách dùng:
#   ./scripts/wipe-history.sh              # dùng data.dir mặc định ~/.bean
#   ./scripts/wipe-history.sh /path/.bean  # chỉ định data dir khác
#
# Script tự: dừng Bean -> backup DB vào ~/.bean/backups/ -> xoá -> kiểm tra -> khởi động lại.
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DATA_DIR="${1:-$HOME/.bean}"
DB="$DATA_DIR/bean.db"

if [ ! -f "$DB" ]; then
    echo "LỖI: không tìm thấy database: $DB" >&2
    exit 1
fi

echo "==> Dừng Bean (phải đóng DB trước khi xoá)..."
"$SCRIPT_DIR/beanctl" stop || true

BACKUP_DIR="$DATA_DIR/backups"
mkdir -p "$BACKUP_DIR"
BACKUP="$BACKUP_DIR/bean.db.$(date +%Y%m%d-%H%M%S).bak"
echo "==> Backup database vào $BACKUP ..."
sqlite3 "$DB" "VACUUM INTO '$BACKUP';"

echo "==> Xoá sessions + messages (CASCADE dọn messages, trigger đồng bộ FTS)..."
sqlite3 "$DB" <<'SQL'
PRAGMA foreign_keys=ON;
BEGIN;
DELETE FROM messages;
DELETE FROM sessions;
COMMIT;
INSERT INTO messages_fts(messages_fts) VALUES('rebuild');
PRAGMA wal_checkpoint(TRUNCATE);
SQL

echo "==> Số dòng còn lại:"
sqlite3 "$DB" "SELECT 'sessions=' || COUNT(*) FROM sessions UNION ALL SELECT 'messages=' || COUNT(*) FROM messages UNION ALL SELECT 'messages_fts=' || COUNT(*) FROM messages_fts;"

echo "==> Khởi động lại Bean..."
"$SCRIPT_DIR/beanctl" start

echo "Xong. Lịch sử chat đã xoá sạch. Backup lưu tại: $BACKUP"
