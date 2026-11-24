//! `PushStore` của [`MemoryStore`] — tách khỏi `memory.rs` để giữ đúng trần số
//! dòng mà test kiến trúc (`tests/architecture.rs`) canh cho thư mục `store/`.
//!
//! Ngữ nghĩa phải **khớp SqliteStore** (`sqlite/q_push.rs`): đăng ký lại là
//! UPSERT giữ nguyên số lỗi, danh sách sắp cũ nhất trước, `record_push_outcome`
//! thành công reset về 0. Test dùng chung trong `tests.rs` chính là ràng buộc này.

use super::memory::MemoryStore;
use super::types::{NewPushSubscription, PushSubscription, StoreError};
use crate::traits::traits::PushStore;

#[async_trait::async_trait]
impl PushStore for MemoryStore {
    async fn upsert_push_subscription(&self, sub: &NewPushSubscription) -> Result<(), StoreError> {
        let mut subs = self.push_subscriptions.write().await;
        match subs.iter_mut().find(|s| s.endpoint == sub.endpoint) {
            // Đăng ký lại: cập nhật key/user-agent, giữ nguyên số lỗi đã đếm —
            // đúng ngữ nghĩa UPSERT của SqliteStore (test dùng chung trong tests.rs).
            Some(existing) => {
                existing.p256dh = sub.p256dh.clone();
                existing.auth = sub.auth.clone();
                existing.user_agent = sub.user_agent.clone();
            }
            None => subs.push(PushSubscription {
                endpoint: sub.endpoint.clone(),
                p256dh: sub.p256dh.clone(),
                auth: sub.auth.clone(),
                user_agent: sub.user_agent.clone(),
                created_at: sub.created_at.clone(),
                failure_count: 0,
            }),
        }
        Ok(())
    }

    async fn list_push_subscriptions(&self) -> Result<Vec<PushSubscription>, StoreError> {
        let mut list: Vec<PushSubscription> = self.push_subscriptions.read().await.clone();
        list.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.endpoint.cmp(&b.endpoint))
        });
        Ok(list)
    }

    async fn delete_push_subscription(&self, endpoint: &str) -> Result<bool, StoreError> {
        let mut subs = self.push_subscriptions.write().await;
        let before = subs.len();
        subs.retain(|s| s.endpoint != endpoint);
        Ok(subs.len() != before)
    }

    async fn record_push_outcome(&self, endpoint: &str, ok: bool) -> Result<(), StoreError> {
        let mut subs = self.push_subscriptions.write().await;
        if let Some(sub) = subs.iter_mut().find(|s| s.endpoint == endpoint) {
            sub.failure_count = if ok {
                0
            } else {
                sub.failure_count.saturating_add(1)
            };
        }
        Ok(())
    }
}
