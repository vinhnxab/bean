//! Restart in-process (M31): yêu cầu `bean serve` tắt êm rồi **exec lại chính
//! nó** — nút "Khởi động lại ngay" kiểu Ubuntu trong banner UI.
//!
//! # Vai trò các phần
//!
//! * [`ProcessRestart`] — điểm mở rộng (DIP) như [`WebRebind`](super::rebind):
//!   `bean serve` cắm handle nối sang select-loop của `serve` (kênh `mpsc`);
//!   test và môi trường nhúng dùng [`NoopRestart`].
//!
//! # Vì sao exec lại thay vì exit + `Restart=always`
//!
//! `exec` giữ nguyên PID chính nên hoạt động **cả hai môi trường**: dưới systemd
//! user unit (journald stream giữ nguyên, `StartLimit` không bị đếm, không mất
//! `RestartSec`) lẫn chạy `nohup` không supervisor (stdout/stderr fd giữ nguyên
//! → log viết tiếp). Tiến trình mới nạp lại `bean.toml` nên thay đổi tier B có
//! hiệu lực. Trước khi exec, `serve` đi hết đường graceful shutdown sẵn có
//! (cancel token → router → web/telegram/scheduler/MCP/Chrome).

/// Điểm kích hoạt restart in-process (M31).
pub trait ProcessRestart: Send + Sync {
    /// Yêu cầu tắt êm rồi chạy lại binary. `true` khi yêu cầu được nhận (lần
    /// đầu); `false` khi một restart khác đã được yêu cầu trước đó và chưa
    /// hoàn tất.
    fn request(&self) -> bool;
}

/// Không-op cho test/môi trường nhúng: nhận yêu cầu nhưng không làm gì —
/// đối xứng [`NoopRebind`](super::rebind::NoopRebind).
pub struct NoopRestart;

impl ProcessRestart for NoopRestart {
    fn request(&self) -> bool {
        true
    }
}
