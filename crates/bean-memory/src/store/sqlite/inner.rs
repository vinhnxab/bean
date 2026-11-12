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

use super::command::{DbCommand, Reply};
use super::migration::{configure, run_migration};
use super::worker::worker_loop;
use crate::store::StoreError;

/// Trần chờ một lệnh store (K3).
///
/// Worker là thread duy nhất chạy SQL, với `busy_timeout` 5 giây (xem
/// `configure`) — mọi lệnh hợp lệ phải xong trong cỡ đó. Vượt ngưỡng này nghĩa
/// là worker kẹt thật: trả lỗi để run kết thúc có kiểm soát thay vì treo mãi
/// (trước khi có trần này, worker kẹt đồng nghĩa mọi lời gọi store treo vĩnh
/// viễn). Giá trị là hằng số cố ý — chưa thấy nhu cầu cấu hình.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Trạng thái dùng chung của [`SqliteStore`] (chia sẻ qua `Arc` để `SqliteStore` clone được).
pub(super) struct SqliteInner {
    tx: Mutex<Option<std::sync::mpsc::Sender<DbCommand>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Trần chờ reply của [`SqliteStore::request`] — mặc định
    /// [`DEFAULT_REQUEST_TIMEOUT`], test dựng trực tiếp với giá trị nhỏ.
    request_timeout: Duration,
}

impl Drop for SqliteInner {
    fn drop(&mut self) {
        // Đóng kênh để `worker_loop` thoát khỏi `recv()`, rồi join để connection được
        // đóng (flush WAL) trước khi tiến trình dừng.
        if let Ok(mut tx) = self.tx.lock() {
            tx.take();
        }
        if let Ok(mut worker) = self.worker.lock()
            && let Some(handle) = worker.take()
            && handle.join().is_err()
        {
            tracing::warn!("worker bộ nhớ kết thúc bất thường");
        }
    }
}

/// Store bền vững trên SQLite + FTS5 (agents.md mục 8).
///
/// `SqliteStore` chỉ giữ kênh gửi lệnh; connection thật nằm ở thread worker riêng nên
/// mọi hàm `async` ở đây đều **không** block runtime.
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

        let (tx, rx) = std::sync::mpsc::channel::<DbCommand>();
        let worker = std::thread::Builder::new()
            .name("bean-memory-worker".to_string())
            .spawn(move || worker_loop(conn, rx))
            .map_err(|err| StoreError::Internal(format!("không tạo được worker bộ nhớ: {err}")))?;

        Ok(Self {
            inner: Arc::new(SqliteInner {
                tx: Mutex::new(Some(tx)),
                worker: Mutex::new(Some(worker)),
                request_timeout: DEFAULT_REQUEST_TIMEOUT,
            }),
        })
    }

    /// Gửi lệnh cho worker và chờ kết quả.
    ///
    /// Chờ reply có trần [`SqliteInner::request_timeout`] (K3): worker kẹt thì
    /// trả [`StoreError::Internal`] thay vì treo mãi — run gọi store được kết
    /// thúc có kiểm soát. Lệnh đã gửi có thể vẫn được worker xử lý xong sau đó;
    /// reply gửi vào receiver đã đóng bị bỏ qua (worker `let _ = send`).
    pub(super) async fn request<T>(
        &self,
        build: impl FnOnce(Reply<T>) -> DbCommand,
    ) -> Result<T, StoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        {
            // Khoá **không** được giữ qua `.await` (agents.md mục 22.7).
            let guard = self
                .inner
                .tx
                .lock()
                .map_err(|_| StoreError::Internal("khoá kênh worker bị hỏng".into()))?;
            let Some(sender) = guard.as_ref() else {
                return Err(StoreError::Internal("store đã đóng".into()));
            };
            sender
                .send(build(reply_tx))
                .map_err(|_| StoreError::Internal("worker bộ nhớ đã dừng".into()))?;
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
    use super::*;

    /// Store "worker kẹt": kênh gửi vẫn sống nhưng không ai trả lời. Dùng để
    /// chứng minh `request` chịu được worker kẹt (K3) — trả lỗi theo trần thay
    /// vì treo vĩnh viễn.
    fn stuck_store(request_timeout: Duration) -> SqliteStore {
        let (tx, rx) = std::sync::mpsc::channel::<DbCommand>();
        // Giữ receiver sống đủ lâu trong thread riêng; tuyệt đối không reply.
        std::thread::spawn(move || {
            let _keep_alive = rx;
            std::thread::sleep(Duration::from_secs(2));
        });
        SqliteStore {
            inner: Arc::new(SqliteInner {
                tx: Mutex::new(Some(tx)),
                worker: Mutex::new(None),
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
}
