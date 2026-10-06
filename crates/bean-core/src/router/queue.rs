//! Bảng điều phối run: hàng đợi theo session + run đang chạy (agents.md mục 10).
//!
//! # Vì sao tách khỏi [`super::Router`]
//!
//! "Một session chạy đúng một run, phần còn lại xếp hàng" là quy tắc **lịch trình thuần
//! tuý**: không I/O, không phát sự kiện, không gọi LLM. Trước đây nó nằm rải rác trong
//! `enqueue` / `finish` / `cancel` / `active_run` / `session_busy` — mỗi hàm tự chạm vào
//! `queues` và `active`, nên thứ tự cập nhật (gỡ `active` **trước** rồi mới
//! `pop_front`) chỉ được bảo đảm bằng mắt thường đọc code.
//!
//! [`RunQueue`] gom toàn bộ chuyển trạng thái ấy vào một chỗ. Nó **không tự khoá**:
//! nhận `&mut self` và luôn được gọi khi `Router` đang giữ `Mutex<RouterState>` — nên
//! không phát sinh thêm thứ tự khoá nào (đây là điều kiện bắt buộc vì `finish` đụng cả
//! hàng đợi lẫn sổ confirm).
//!
//! Việc *phát sự kiện* (`Queued`, `ConfirmResolved`) và *spawn task* vẫn thuộc
//! `Router`: queue chỉ trả về kết quả để Router phát ra ngoài.

use std::collections::{HashMap, VecDeque};

use bean_types::{RunId, SessionId};
use tokio_util::sync::CancellationToken;

use super::{QueuedRun, RunningInfo};

/// Hàng đời của một session: run đang chạy + các run chờ.
#[derive(Debug)]
struct SessionQueue {
    /// Run đang chạy; `None` ⇒ session rảnh.
    active: Option<RunId>,
    /// Các run chờ, theo thứ tự xếp.
    pending: VecDeque<QueuedRun>,
}

/// Một run đang chạy (đã spawn).
#[derive(Debug, Clone)]
pub(super) struct ActiveRun {
    /// Session sở hữu run — lưu kèm để `running()` không phải tra cứu ngược.
    pub session: SessionId,
    /// Kênh sở hữu run.
    pub channel: String,
    /// Chat sở hữu run.
    pub chat_id: String,
    /// Token huỷ (nút Dừng / `/stop` / shutdown).
    pub cancel: CancellationToken,
    /// Role đã resolve cho run này (M21.3) — để HUB quy đổi "đang chạy" về đúng
    /// agent thay vì chỉ biết có run. `None` khi RBAC tắt.
    pub role: Option<String>,
}

/// Kết quả [`RunQueue::enqueue`].
pub(super) enum Enqueued {
    /// Session rảnh ⇒ chạy ngay.
    Start,
    /// Đã xếp hàng ở vị trí này (1-based).
    Queued {
        /// Vị trí trong hàng đợi, tính từ 1.
        position: u32,
    },
}

/// Kết quả [`RunQueue::advance`] khi một run kết thúc.
pub(super) struct Advanced {
    /// Run kế tiếp được kích hoạt (nếu có).
    pub next: Option<QueuedRun>,
    /// Vị trí mới của các run còn chờ, để phát lại `RunEvent::Queued`.
    pub positions: Vec<(RunId, u32)>,
}

/// Hàng đợi theo session + run đang chạy.
#[derive(Debug, Default)]
pub(super) struct RunQueue {
    /// Hàng đợi theo session.
    queues: HashMap<SessionId, SessionQueue>,
    /// Run đang chạy, tra theo `run_id`.
    active: HashMap<RunId, ActiveRun>,
}

impl RunQueue {
    /// Đưa một run vào hàng đợi của session và trả về việc cần làm tiếp.
    ///
    /// # Panics
    /// Không — hàm thuần tuý trên trạng thái, trả [`Enqueued`] để caller quyết định
    /// phát sự kiện / spawn.
    pub(super) fn enqueue(&mut self, queued: &QueuedRun) -> Enqueued {
        let queue = self
            .queues
            .entry(queued.session_id)
            .or_insert_with(|| SessionQueue {
                active: None,
                pending: VecDeque::new(),
            });
        if queue.active.is_some() {
            queue.pending.push_back(queued.clone());
            Enqueued::Queued {
                position: queue.pending.len() as u32,
            }
        } else {
            queue.active = Some(queued.run_id.clone());
            self.mark_active(queued);
            Enqueued::Start
        }
    }

    /// Ghi một run vào bảng active (dùng khi bắt đầu hoặc khi kích hoạt run kế tiếp).
    fn mark_active(&mut self, queued: &QueuedRun) {
        self.active.insert(
            queued.run_id.clone(),
            ActiveRun {
                session: queued.session_id,
                channel: queued.incoming.channel.clone(),
                chat_id: queued.incoming.chat_id.clone(),
                cancel: queued.cancel.clone(),
                role: queued.role.clone(),
            },
        );
    }

    /// Session còn run đang chạy, hoặc còn việc chờ.
    pub(super) fn is_busy(&self, session: SessionId) -> bool {
        self.queues
            .get(&session)
            .is_some_and(|queue| queue.active.is_some() || !queue.pending.is_empty())
    }

    /// Gỡ run vừa kết thúc khỏi bảng active và kích hoạt run kế tiếp của cùng session.
    ///
    /// `shutting_down` ⇒ **xoá** phần còn chờ thay vì chạy tiếp (agents.md mục 10).
    ///
    /// Trả `None` khi `run_id` **không** phải run đang active của `session` — nghĩa là
    /// lời gọi đến muộn hoặc không hợp lệ; trường hợp này tuyệt đối không được đụng vào
    /// hàng đợi, vì sẽ kích hoạt nhầm một run khác đang chờ.
    pub(super) fn advance(
        &mut self,
        run_id: &RunId,
        session: SessionId,
        shutting_down: bool,
    ) -> Option<Advanced> {
        self.active.remove(run_id);
        let (next, positions) = {
            let queue = self.queues.get_mut(&session)?;
            if queue.active.as_ref() != Some(run_id) {
                return None;
            }
            queue.active = None;
            let next = if shutting_down {
                queue.pending.clear();
                None
            } else {
                queue.pending.pop_front()
            };
            let positions: Vec<_> = queue
                .pending
                .iter()
                .enumerate()
                .map(|(index, job)| (job.run_id.clone(), index as u32 + 1))
                .collect();
            if let Some(next) = &next {
                queue.active = Some(next.run_id.clone());
            }
            (next, positions)
        };
        if let Some(next) = &next {
            self.mark_active(next);
        }
        Some(Advanced { next, positions })
    }

    /// Token huỷ của run đang chạy trên `channel`/`chat_id`.
    pub(super) fn cancel_token_for_channel(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Option<CancellationToken> {
        self.active
            .iter()
            .find(|(_, active)| active.channel == channel && active.chat_id == chat_id)
            .map(|(_, active)| active.cancel.clone())
    }

    /// Token huỷ của run đang chạy của đúng một session.
    ///
    /// Tách khỏi `cancel_token_for_channel` là cố ý: adapter web chỉ có `session_id`,
    /// và nhiều session của cùng user dùng chung `(channel, chat_id)` — huỷ theo
    /// channel/chat_id sẽ dừng nhầm session khác.
    pub(super) fn cancel_token_for_session(&self, session: SessionId) -> Option<CancellationToken> {
        let run_id = self.queues.get(&session)?.active.clone()?;
        self.active.get(&run_id).map(|active| active.cancel.clone())
    }

    /// Run đang chạy của `channel`/`chat_id`.
    pub(super) fn active_run_for(&self, channel: &str, chat_id: &str) -> Option<RunId> {
        self.active
            .iter()
            .find(|(_, active)| active.channel == channel && active.chat_id == chat_id)
            .map(|(run_id, _)| run_id.clone())
    }

    /// Role của run đang chạy — HUB dùng để gắn confirm về đúng agent.
    pub(super) fn role_of(&self, run_id: &RunId) -> Option<String> {
        self.active.get(run_id)?.role.clone()
    }

    /// Mọi token của run đang chạy lẫn run đang chờ — dùng khi shutdown.
    pub(super) fn all_cancel_tokens(&self) -> Vec<CancellationToken> {
        let mut tokens: Vec<_> = self.active.values().map(|run| run.cancel.clone()).collect();
        tokens.extend(
            self.queues
                .values()
                .flat_map(|queue| queue.pending.iter().map(|job| job.cancel.clone())),
        );
        tokens
    }

    /// Danh sách run đang chạy, đọc nguyên tử cho `Sync`.
    ///
    /// `session` đọc thẳng từ [`ActiveRun`] thay vì quét ngược `queues` để tìm session
    /// đang giữ `run_id`. Cách cũ tốn O(số queue) cho mỗi run **và** phải có nhánh lỗi:
    /// khi không tìm thấy thì rơi về `SessionId::new(0)` — một session **giả** lọt
    /// thẳng ra `Sync`, khiến client gắn run của session này vào session 0. Nay
    /// không còn nhánh lỗi vì dữ liệu nằm ngay chỗ sinh ra nó.
    pub(super) fn running(&self) -> Vec<RunningInfo> {
        self.active
            .iter()
            .map(|(run_id, active)| RunningInfo {
                run_id: run_id.clone(),
                session_id: active.session,
                role: active.role.clone(),
            })
            .collect()
    }
}
