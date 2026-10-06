//! Schema SQLite và pragma (agents.md mục 8.1).
//!
//! # Vì sao tách riêng
//!
//! Đây là **hợp đồng với đĩa**: các câu `CREATE TABLE`, chỉ mục, trigger đồng bộ FTS5 và
//! `PRAGMA`. Nó là thứ đọc khi debug "sai ở đâu", và là thứ **không ai nên sửa** khi
//! thêm một method vào trait — nên nó không được nằm chung với code đọc/ghi bảng.
//!
//! `SCHEMA_SQL` idempotent (`IF NOT EXISTS`) để chạy lại vô hại; migration tiến hoá
//! nằm ở `migration.rs`, không nhét vào đây.

/// Schema v1 (agents.md mục 8.1). Idempotent để chạy lại an toàn.
pub(crate) const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS sessions (
  id INTEGER PRIMARY KEY,
  channel TEXT NOT NULL,
  chat_id TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  archived INTEGER NOT NULL DEFAULT 0,
  summary TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_chat ON sessions(channel, chat_id, archived);

CREATE TABLE IF NOT EXISTS messages (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  role TEXT NOT NULL,
  content_json TEXT NOT NULL,
  text_for_search TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  UNIQUE(session_id, seq)
);
CREATE INDEX IF NOT EXISTS messages_session ON messages(session_id, seq);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
  text_for_search,
  content='messages',
  content_rowid='id',
  tokenize = \"unicode61 remove_diacritics 2\"
);
CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, text_for_search) VALUES (new.id, new.text_for_search);
END;
CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text_for_search) VALUES ('delete', old.id, old.text_for_search);
END;
CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text_for_search) VALUES ('delete', old.id, old.text_for_search);
  INSERT INTO messages_fts(rowid, text_for_search) VALUES (new.id, new.text_for_search);
END;

CREATE TABLE IF NOT EXISTS memories (
  id INTEGER PRIMARY KEY,
  text TEXT NOT NULL,
  tags TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
  text,
  tags,
  content='memories',
  content_rowid='id',
  tokenize = \"unicode61 remove_diacritics 2\"
);
CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
  INSERT INTO memories_fts(rowid, text, tags) VALUES (new.id, new.text, new.tags);
END;
CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, text, tags) VALUES ('delete', old.id, old.text, old.tags);
END;
CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, text, tags) VALUES ('delete', old.id, old.text, old.tags);
  INSERT INTO memories_fts(rowid, text, tags) VALUES (new.id, new.text, new.tags);
END;

CREATE TABLE IF NOT EXISTS scheduled_tasks (
  id INTEGER PRIMARY KEY,
  cron TEXT NOT NULL,
  prompt TEXT NOT NULL,
  channel TEXT NOT NULL,
  chat_id TEXT NOT NULL,
  allowed_tools TEXT NOT NULL DEFAULT '[]',
  next_run TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS outbox (
  id INTEGER PRIMARY KEY,
  channel TEXT NOT NULL,
  chat_id TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS web_sessions (
  token_hash BLOB PRIMARY KEY,
  user_id TEXT NOT NULL DEFAULT 'web:admin',
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  last_seen TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS usage (
  day TEXT PRIMARY KEY,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0
);
-- M21.7: ngân sách token tách riêng theo từng role, tách khỏi bảng `usage` tổng
-- (bảng `usage` vẫn phục vụ `GET /api/status`).
CREATE TABLE IF NOT EXISTS usage_by_role (
  day TEXT NOT NULL,
  role TEXT NOT NULL,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, role)
);
CREATE INDEX IF NOT EXISTS web_sessions_expiry ON web_sessions(expires_at);
-- M25: token client MCP. Chỉ lưu HASH token (không lưu token thô), `expires_at` rỗng
-- ⇒ không hết hạn. `name` là khoá chính xác để cấp lại/thu hồi theo tên client.
CREATE TABLE IF NOT EXISTS mcp_clients (
  token_hash TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  role TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL DEFAULT ''
);
";

/// Pragma bắt buộc (agents.md mục 8.1): WAL cho đọc/ghi song song, khoá ngoại bật.
pub(super) const PRAGMA_SQL: &str = "\
PRAGMA journal_mode = WAL;\
PRAGMA foreign_keys = ON;\
PRAGMA synchronous = NORMAL;";
