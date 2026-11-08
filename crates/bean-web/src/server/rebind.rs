//! Hot rebind listener web (D21.7): đổi `web.bind`/`web.public_origin`/
//! `web.allow_remote` **lúc nóng** khi LAN toggle ghi `bean.toml` từ UI — không
//! cần `beanctl restart`, run agent đang chạy không bị gián đoạn.
//!
//! # Vai trò các phần
//!
//! * [`WebListen`] — bộ ba tham số lắng nghe hiệu lực, đọc nóng bởi guard/handler
//!   qua `watch` cell trong [`WebState`](super::WebState).
//! * [`WebRebind`] — điểm mở rộng áp trạng thái mới (DIP): `bean serve` cắm
//!   [`RebindHandle`] nối sang [`run_web_supervisor`]; test/môi trường không
//!   supervisor dùng [`NoopRebind`].
//! * [`run_web_supervisor`] — sở hữu listener: dừng cũ (graceful có trần) → bind
//!   mới → swap; bind mới thất bại thì khôi phục listener cũ, web không đứng im.
//!
//! # Ranh giới bảo mật
//!
//! Guard đọc origin hiệu lực từng request — ngay sau swap, request mang origin cũ
//! bị từ chối (403) và UI tự nối lại WS + nạp lại bằng REST (cơ chế có sẵn từ
//! M10/M16). Người dùng phải mở URL mới (hiển thị trong `enableHint`/card LAN).

use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use url::Url;

use bean_types::Config;

use super::WebBuildError;
use super::WebState;
use super::routes::build_router;

/// Trần chờ graceful shutdown của listener cũ trước khi abort — kết nối WS mở
/// (tab PWA) sẽ bị cắt và client tự nối lại với backoff (đã test từ M10/M16).
const REBIND_GRACE: Duration = Duration::from_secs(2);
/// Số lần thử bind lại listener cũ khi khôi phục sau bind mới thất bại.
const RESTORE_ATTEMPTS: u32 = 3;

/// Bộ ba tham số lắng nghe web hiệu lực — một đơn vị swap nguyên tử (D21.2).
#[derive(Clone, Debug)]
pub struct WebListen {
    /// Địa chỉ bind hiện hành.
    pub bind: SocketAddr,
    /// Cờ cho phép bind ngoài loopback.
    pub allow_remote: bool,
    /// Origin canonical dùng cho guard CSRF/CSWSH.
    pub public_origin: Url,
}

impl WebListen {
    /// Dựng từ cấu hình lúc khởi động; origin sai trả
    /// [`WebBuildError::InvalidOrigin`] (cùng bất biến với `WebState::new`).
    pub fn from_config(config: &Config) -> Result<Self, WebBuildError> {
        Ok(Self {
            bind: config.web.bind,
            allow_remote: config.web.allow_remote,
            public_origin: validate_origin(&config.web.public_origin)?,
        })
    }
}

/// Kiểm tra origin canonical: scheme http/https, có host, không userinfo/path
/// (ngoài `/`)/query/fragment.
pub(crate) fn validate_origin(raw: &str) -> Result<Url, WebBuildError> {
    let origin = Url::parse(raw).map_err(|_| WebBuildError::InvalidOrigin)?;
    if !matches!(origin.scheme(), "http" | "https")
        || origin.host_str().is_none()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(WebBuildError::InvalidOrigin);
    }
    Ok(origin)
}

/// Điểm mở rộng áp trạng thái lắng nghe mới lúc nóng (D21.7). Implement phải
/// swap listener **trước khi** trả `Ok` — `Ok` nghĩa là web đang phục vụ trên
/// `listen.bind` với origin mới; `Err` nghĩa là bind mới thất bại và listener cũ
/// đã được khôi phục (web không bao giờ đứng im không listener).
#[async_trait]
pub trait WebRebind: Send + Sync {
    /// Áp trạng thái lắng nghe mới.
    async fn apply(&self, listen: WebListen) -> Result<(), String>;
}

/// Không-op cho test và nhúng không supervisor: báo thành công mà không đổi gì.
pub struct NoopRebind;

#[async_trait]
impl WebRebind for NoopRebind {
    async fn apply(&self, _listen: WebListen) -> Result<(), String> {
        Ok(())
    }
}

/// [`WebRebind`] cắm vào [`WebState`] khi có supervisor: gửi yêu cầu qua mpsc và
/// chờ kết quả thật (bind thành công/thất bại) để phản hồi đúng cho người dùng.
pub struct RebindHandle {
    tx: mpsc::Sender<(WebListen, oneshot::Sender<Result<(), String>>)>,
}

impl RebindHandle {
    /// Tạo handle gửi yêu cầu sang [`run_web_supervisor`] đang giữ `rx`.
    #[must_use]
    pub fn new(tx: mpsc::Sender<(WebListen, oneshot::Sender<Result<(), String>>)>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl WebRebind for RebindHandle {
    async fn apply(&self, listen: WebListen) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send((listen, reply))
            .await
            .map_err(|_| "supervisor web đã dừng".to_string())?;
        rx.await
            .map_err(|_| "supervisor web đã dừng khi chờ kết quả rebind".to_string())?
    }
}

/// Vòng lặp supervisor: giữ listener hiện hành, swap lúc nóng theo yêu cầu của
/// [`RebindHandle`], khôi phục listener cũ khi bind mới thất bại, tắt êm theo
/// `shutdown`. `serve.rs` spawn task này một lần lúc khởi động; cell
/// `web_listen` của state được cập nhật sau mỗi lần swap thành công để guard
/// đọc đúng origin hiệu lực.
///
/// Trả `Err` khi serve task **chết bất ngờ** (không do token rebind/shutdown) —
/// cùng ngữ nghĩa với `axum::serve` lỗi trước đây: `serve.rs` thoát tiến trình để
/// systemd/beanctl restart, thay vì đứng im với listener chết.
pub async fn run_web_supervisor(
    state: WebState,
    initial: (TcpListener, WebListen),
    mut rx: mpsc::Receiver<(WebListen, oneshot::Sender<Result<(), String>>)>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let mut current_listen = initial.1;
    let (mut current_handle, mut current_stop) = spawn_serve(&state, initial.0, &shutdown);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                stop_serve(&mut current_handle, &current_stop).await;
                return Ok(());
            }
            request = rx.recv() => {
                let Some((target, reply)) = request else {
                    // Handle đã drop — không còn ai yêu cầu rebind: đợi shutdown.
                    shutdown.cancelled().await;
                    stop_serve(&mut current_handle, &current_stop).await;
                    return Ok(());
                };
                let outcome = swap_listener(
                    &state,
                    &mut current_handle,
                    &mut current_stop,
                    &mut current_listen,
                    target,
                    &shutdown,
                )
                .await;
                let _ = reply.send(outcome);
            }
            // Serve task kết thúc khi cả hai token đều chưa bấm = chết bất ngờ.
            died = &mut current_handle => {
                if shutdown.is_cancelled() {
                    return Ok(());
                }
                anyhow::bail!("web server dừng bất ngờ trên {}: {died:?}", current_listen.bind);
            }
        }
    }
}

/// Dừng listener cũ → bind mới → swap. Bind mới thất bại thì khôi phục listener
/// cũ (thử lại [`RESTORE_ATTEMPTS`] lần) và trả `Err` — handler sẽ phản hồi
/// `restart_required = true`, restart sẽ áp cấu hình đã ghi đĩa.
async fn swap_listener(
    state: &WebState,
    current_handle: &mut JoinHandle<()>,
    current_stop: &mut CancellationToken,
    current_listen: &mut WebListen,
    target: WebListen,
    shutdown: &CancellationToken,
) -> Result<(), String> {
    stop_serve(current_handle, current_stop).await;
    match TcpListener::bind(target.bind).await {
        Ok(listener) => {
            let (handle, stop) = spawn_serve(state, listener, shutdown);
            *current_handle = handle;
            *current_stop = stop;
            *current_listen = target;
            state.web_listen.send_modify(|cell| {
                *cell = current_listen.clone();
            });
            Ok(())
        }
        Err(error) => {
            warn!(bind = %target.bind, %error, "rebind thất bại — khôi phục listener cũ");
            for _ in 0..RESTORE_ATTEMPTS {
                match TcpListener::bind(current_listen.bind).await {
                    Ok(listener) => {
                        let (handle, stop) = spawn_serve(state, listener, shutdown);
                        *current_handle = handle;
                        *current_stop = stop;
                        return Err(format!(
                            "không bind được {} ({error}) — đã khôi phục {}",
                            target.bind, current_listen.bind
                        ));
                    }
                    Err(restore_error) => {
                        warn!(bind = %current_listen.bind, %restore_error, "khôi phục listener thất bại");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
            Err(format!(
                "không bind được {} ({error}) và không khôi phục được {} — web đang dừng, restart để áp cấu hình",
                target.bind, current_listen.bind
            ))
        }
    }
}

/// Spawn task `axum::serve` cho một listener; dừng khi token riêng (rebind) hoặc
/// token toàn cục (shutdown) bấm. Origin hiệu lực không nằm trong task — guard
/// đọc từ cell `web_listen` của state nên swap không phải dựng lại gì thêm.
fn spawn_serve(
    state: &WebState,
    listener: TcpListener,
    shutdown: &CancellationToken,
) -> (JoinHandle<()>, CancellationToken) {
    let stop = CancellationToken::new();
    let global = shutdown.clone();
    let stop_for_serve = stop.clone();
    let router = build_router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                tokio::select! {
                    _ = stop_for_serve.cancelled() => {}
                    _ = global.cancelled() => {}
                }
            })
            .await;
    });
    (handle, stop)
}

/// Dừng listener hiện hành: graceful có trần [`REBIND_GRACE`] rồi abort — kết nối
/// WS mở bị cắt, client tự nối lại (Sync khôi phục trạng thái run/confirm).
async fn stop_serve(current_handle: &mut JoinHandle<()>, current_stop: &CancellationToken) {
    current_stop.cancel();
    if tokio::time::timeout(REBIND_GRACE, &mut *current_handle)
        .await
        .is_err()
    {
        current_handle.abort();
    }
}
