//! Trạng thái chia sẻ và vòng đời của [`SqliteStore`] (agents.md mục 22.8).
//!
//! # Vì sao tách riêng
//!
//! `SqliteStore` chỉ giữ **kênh gửi lệnh**; connection thật nằm ở worker thread nên mọi
//! hàm `async` ở đây đều không block runtime. Ba thứ trong file này — [`SqliteInner`],
//! [`Drop`] và `open`/`request` — là **vòng đời** của worker, tách bạch khỏi "lệnh nào
//! thì chạy SQL gì".
//!
//! Phần `impl Store` (đóng gói 51 lệnh) nằm ở `mod.rs`: giữ nó cùng chỗ với worker
//! để đọc một lần là thấy hết đường đi của một lời gọi từ `async` tới SQL.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use tokio::sync::oneshot;

use super::command::{CmdKind, DbCommand, Reply};
use super::migration::{configure, configure_reader, run_migration};
use super::worker::worker_loop;
use crate::store::StoreError;

/// Trần chờ một lệnh store (K3).
///
/// Worker là thread chạy SQL, với `busy_timeout` 5 giây (xem `configure`) —
/// mọi lệnh hợp lệ phải xong trong cỡ đó. Vượt ngưỡng này nghĩa là worker kẹt
/// thật: trả lỗi để run kết thúc có kiểm soát thay vì treo mãi (trước khi có
/// trần này, worker kẹt đồng nghĩa mọi lời gọi store treo vĩnh viễn). Giá trị
/// là hằng số cố ý — chưa thấy nhu cầu cấu hình.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Hai kênh gửi lệnh của [`SqliteStore`] (K4): writer nhận lệnh ghi, reader
/// nhận lệnh đọc. Định tuyến ở [`SqliteStore::request`] theo [`CmdKind`].
struct Channels {
    write: Mutex<Option<std::sync::mpsc::Sender<DbCommand>>>,
    read: Mutex<Option<std::sync::mpsc::Sender<DbCommand>>>,
}

/// Trạng thái dùng chung của [`SqliteStore`] (chia sẻ qua `Arc` để `SqliteStore` clone được).
pub(super) struct SqliteInner {
    channels: Channels,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    reader: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Trần chờ reply của [`SqliteStore::request`] — mặc định
    /// [`DEFAULT_REQUEST_TIMEOUT`], test dựng trực tiếp với giá trị nhỏ.
    request_timeout: Duration,
}

impl Drop for SqliteInner {
    fn drop(&mut self) {
        // Đóng cả hai kênh để hai worker thoát khỏi `recv()`, rồi join để
        // connection được đóng (flush WAL) trước khi tiến trình dừng.
        if let Ok(mut write) = self.channels.write.lock() {
            write.take();
        }
        if let Ok(mut read) = self.channels.read.lock() {
            read.take();
        }
        for (name, lock) in [("worker", &self.worker), ("reader", &self.reader)] {
            if let Ok(mut handle) = lock.lock()
                && let Some(handle) = handle.take()
                && handle.join().is_err()
            {
                tracing::warn!("worker bộ nhớ ({name}) kết thúc bất thường");
            }
        }
    }
}

/// Store bền vững trên SQLite + FTS5 (agents.md mục 8).
///
/// `SqliteStore` chỉ giữ kênh gửi lệnh; connection thật nằm ở **hai** thread
/// worker riêng (K4): writer giữ connection ghi, reader giữ connection chỉ đọc
/// (`query_only`). WAL cho phép reader chạy song song với writer nên truy vấn
/// FTS nặng không chặn `append`. Mọi hàm `async` ở đây đều **không** block runtime.
#[derive(Clone)]
pub struct SqliteStore {
    inner: Arc<SqliteInner>,
}

impl SqliteStore {
    /// Mở (hoặc tạo) database ở `path`, chạy migration rồi khởi động worker thread.
    ///
    /// # Errors
    /// [`StoreError::Internal`] khi không tạo được thư mục/mở file/migration lỗi hoặc
    /// không tạo được worker.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                StoreError::Internal(format!(
                    "tạo thư mục dữ liệu {} thất bại: {err}",
                    parent.display()
                ))
            })?;
        }
        let conn = Connection::open(path).map_err(|err| {
            StoreError::Internal(format!("mở SQLite {} thất bại: {err}", path.display()))
        })?;
        configure(&conn)?;
        run_migration(&conn)?;

        // Reader mở sau migration để schema chắc chắn sẵn sàng; WAL là thuộc tính
        // bền của file nên reader tự nhận chế độ WAL của writer.
        let reader_conn = Connection::open(path).map_err(|err| {
            StoreError::Internal(format!(
                "mở SQLite reader {} thất bại: {err}",
                path.display()
            ))
        })?;
        configure_reader(&reader_conn)?;

        let (write_tx, write_rx) = std::sync::mpsc::channel::<DbCommand>();
        let (read_tx, read_rx) = std::sync::mpsc::channel::<DbCommand>();
        let worker = std::thread::Builder::new()
            .name("bean-memory-worker".to_string())
            .spawn(move || worker_loop(conn, write_rx))
            .map_err(|err| StoreError::Internal(format!("không tạo được worker bộ nhớ: {err}")))?;
        let reader = std::thread::Builder::new()
            .name("bean-memory-reader".to_string())
            .spawn(move || worker_loop(reader_conn, read_rx))
            .map_err(|err| StoreError::Internal(format!("không tạo được reader bộ nhớ: {err}")))?;

        Ok(Self {
            inner: Arc::new(SqliteInner {
                channels: Channels {
                    write: Mutex::new(Some(write_tx)),
                    read: Mutex::new(Some(read_tx)),
                },
                worker: Mutex::new(Some(worker)),
                reader: Mutex::new(Some(reader)),
                request_timeout: DEFAULT_REQUEST_TIMEOUT,
            }),
        })
    }

    /// Gửi lệnh cho worker phù hợp và chờ kết quả (K4).
    ///
    /// Định tuyến theo [`CmdKind`]: lệnh đọc qua reader, lệnh ghi qua writer —
    /// phân loại là `match` vét cạn nên thêm biến thể mới buộc phải chọn phía.
    /// Chờ reply có trần [`SqliteInner::request_timeout`] (K3): worker kẹt thì
    /// trả [`StoreError::Internal`] thay vì treo mãi — run gọi store được kết
    /// thúc có kiểm soát. Lệnh đã gửi có thể vẫn được worker xử lý xong sau đó;
    /// reply gửi vào receiver đã đóng bị bỏ qua (worker `let _ = send`).
    pub(super) async fn request<T>(
        &self,
        build: impl FnOnce(Reply<T>) -> DbCommand,
    ) -> Result<T, StoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = build(reply_tx);
        {
            // Khoá **không** được giữ qua `.await` (agents.md mục 22.7).
            let (lock, side) = match cmd.kind() {
                CmdKind::Read => (&self.inner.channels.read, "reader"),
                CmdKind::Write => (&self.inner.channels.write, "writer"),
            };
            let guard = lock
                .lock()
                .map_err(|_| StoreError::Internal("khoá kênh worker bị hỏng".into()))?;
            let Some(sender) = guard.as_ref() else {
                return Err(StoreError::Internal("store đã đóng".into()));
            };
            sender
                .send(cmd)
                .map_err(|_| StoreError::Internal(format!("worker bộ nhớ ({side}) đã dừng")))?;
        }
        match tokio::time::timeout(self.inner.request_timeout, reply_rx).await {
            Ok(reply) => {
                reply.map_err(|_| StoreError::Internal("worker bộ nhớ không phản hồi".into()))?
            }
            Err(_) => Err(StoreError::Internal(format!(
                "store không phản hồi sau {} giây — worker bộ nhớ kẹt (K3); \
                 nếu lặp lại, khởi động lại Bean",
                self.inner.request_timeout.as_secs()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    // Test thì được phép unwrap/expect — nguyên tắc "không panic trong code sản
    // phẩm" (agents.md mục 0.8) không áp dụng cho test.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use bean_types::SessionId;

    /// Store "worker kẹt": cả hai kênh gửi vẫn sống nhưng không ai trả lời. Dùng
    /// để chứng minh `request` chịu được worker kẹt (K3) — trả lỗi theo trần thay
    /// vì treo vĩnh viễn. Lệnh đọc (ToolOverrides) được định tuyến qua kênh đọc.
    fn stuck_store(request_timeout: Duration) -> SqliteStore {
        let (write_tx, write_rx) = std::sync::mpsc::channel::<DbCommand>();
        let (read_tx, read_rx) = std::sync::mpsc::channel::<DbCommand>();
        // Giữ receiver sống đủ lâu trong thread riêng; tuyệt đối không reply.
        std::thread::spawn(move || {
            let _keep_alive = (write_rx, read_rx);
            std::thread::sleep(Duration::from_secs(2));
        });
        SqliteStore {
            inner: Arc::new(SqliteInner {
                channels: Channels {
                    write: Mutex::new(Some(write_tx)),
                    read: Mutex::new(Some(read_tx)),
                },
                worker: Mutex::new(None),
                reader: Mutex::new(None),
                request_timeout,
            }),
        }
    }

    #[tokio::test]
    async fn request_returns_error_when_worker_never_replies() {
        let store = stuck_store(Duration::from_millis(50));
        let started = std::time::Instant::now();
        let error = store
            .request(|reply| DbCommand::ToolOverrides { reply })
            .await
            .expect_err("worker kẹt phải trả lỗi, không treo");
        let message = error.to_string();
        assert!(
            message.contains("không phản hồi sau"),
            "lỗi phải nói rõ worker không phản hồi: {message}"
        );
        // Trần 50ms được tôn trọng (không chờ 2s của thread giữ receiver).
        assert!(
            started.elapsed() < Duration::from_millis(2_000),
            "request phải trả lỗi theo trần, không chờ receiver đóng"
        );
    }

    /// Cơ chế mà K4 dựa vào: reader giữ read-txn mở **không** được chặn writer.
    /// Nếu ai đó trả journal về rollback (mất WAL), `writer.execute` dưới đây sẽ
    /// kẹt busy_timeout 5s rồi lỗi — test này là canary cho toàn bộ thiết kế.
    #[test]
    fn reader_holding_read_txn_does_not_block_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.db");
        let writer = Connection::open(&path).unwrap();
        configure(&writer).unwrap();
        run_migration(&writer).unwrap();
        let reader = Connection::open(&path).unwrap();
        configure_reader(&reader).unwrap();

        reader.execute_batch("BEGIN;").unwrap();
        reader
            .prepare("SELECT count(*) FROM messages")
            .unwrap()
            .query_row([], |row| row.get::<_, i64>(0))
            .unwrap();

        let wrote = writer.execute(
            "INSERT INTO memories (text, tags, created_at) \
             VALUES ('k4-canary', '', '2026-01-01T00:00:00Z')",
            [],
        );
        reader.execute_batch("COMMIT;").unwrap();
        wrote.expect("ghi phải xong ngay khi reader giữ read-txn (WAL)");
    }

    /// Reader phải bật `query_only`: lệnh ghi lọt reader phải lỗi lộ ngay thay
    /// vì im lặng ghi lệch — đây là lưới an toàn cho việc phân loại `CmdKind`.
    #[test]
    fn reader_connection_rejects_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ro.db");
        let writer = Connection::open(&path).unwrap();
        configure(&writer).unwrap();
        run_migration(&writer).unwrap();
        let reader = Connection::open(&path).unwrap();
        configure_reader(&reader).unwrap();
        let error = reader
            .execute(
                "INSERT INTO memories (text, tags, created_at) \
                 VALUES ('x', '', '2026-01-01T00:00:00Z')",
                [],
            )
            .expect_err("reader bật query_only phải từ chối ghi");
        assert!(
            error.to_string().to_lowercase().contains("readonly"),
            "lỗi phải nói rõ connection chỉ đọc: {error}"
        );
    }

    /// Phân loại `CmdKind` cho các lệnh đại diện cả hai phía. Thêm biến thể mới
    /// sẽ bị `kind()` vét cạn ép phải chọn; chọn sai chiều đọc→ghi thì bị hai
    /// test connection (`query_only`) bắt trong test store, chọn sai chiều
    /// ghi→đọc thì các lệnh nặng như `MemorySearch` âm thầm dồn về writer —
    /// mất ý nghĩa K4, nên những lệnh đọc nặng nhất được liệt kê tường minh ở đây.
    #[test]
    fn heavy_reads_route_to_reader_and_writes_route_to_writer() {
        let session = SessionId::new(1);
        let reads: Vec<DbCommand> = vec![
            DbCommand::History {
                session,
                before_seq: None,
                limit: 1,
                reply: oneshot::channel().0,
            },
            DbCommand::Count {
                session,
                reply: oneshot::channel().0,
            },
            DbCommand::ListSessions {
                user_id: "web:admin".into(),
                query: None,
                archived: None,
                limit: 1,
                reply: oneshot::channel().0,
            },
            DbCommand::ListMessages {
                session,
                before_seq: None,
                limit: 1,
                reply: oneshot::channel().0,
            },
            DbCommand::ListMemories {
                query: None,
                limit: 1,
                reply: oneshot::channel().0,
            },
            DbCommand::ListTasks {
                reply: oneshot::channel().0,
            },
            DbCommand::DueOutbox {
                now: "2026-01-01T00:00:00Z".into(),
                limit: 1,
                reply: oneshot::channel().0,
            },
            DbCommand::ToolOverrides {
                reply: oneshot::channel().0,
            },
            DbCommand::MemorySearch {
                query: "k4".into(),
                exclude_current_run_of: None,
                reply: oneshot::channel().0,
            },
        ];
        for cmd in &reads {
            assert_eq!(
                cmd.kind(),
                CmdKind::Read,
                "lệnh đọc phải qua reader: {cmd:?}"
            );
        }
        let writes: Vec<DbCommand> = vec![
            DbCommand::Append {
                session,
                message: bean_types::Message::user("k4"),
                reply: oneshot::channel().0,
            },
            DbCommand::EnsureSession {
                channel: "web".into(),
                chat_id: "c".into(),
                user_id: "web:admin".into(),
                title: String::new(),
                reply: oneshot::channel().0,
            },
            DbCommand::ClaimTask {
                id: 1,
                expected_next_run: "2026-01-01T00:00:00Z".into(),
                next_run: "2026-01-01T00:00:00Z".into(),
                last_run_at: "2026-01-01T00:00:00Z".into(),
                status: "running".into(),
                reply: oneshot::channel().0,
            },
            DbCommand::DeleteTask {
                id: 1,
                reply: oneshot::channel().0,
            },
            DbCommand::DeleteBefore {
                session,
                before_seq: 1,
                reply: oneshot::channel().0,
            },
        ];
        for cmd in &writes {
            assert_eq!(
                cmd.kind(),
                CmdKind::Write,
                "lệnh ghi phải qua writer: {cmd:?}"
            );
        }
    }
}
