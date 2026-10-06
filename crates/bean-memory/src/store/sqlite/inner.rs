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

use rusqlite::Connection;
use tokio::sync::oneshot;

use super::command::{DbCommand, Reply};
use super::migration::{configure, run_migration};
use super::worker::worker_loop;
use crate::store::StoreError;

/// Trạng thái dùng chung của [`SqliteStore`] (chia sẻ qua `Arc` để `SqliteStore` clone được).
pub(super) struct SqliteInner {
    tx: Mutex<Option<std::sync::mpsc::Sender<DbCommand>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
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
            }),
        })
    }

    /// Gửi lệnh cho worker và chờ kết quả.
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
        reply_rx
            .await
            .map_err(|_| StoreError::Internal("worker bộ nhớ không phản hồi".into()))?
    }
}
