//! Cổng giới hạn tần suất learning loop (M15, agents.md mục 17).
//!
//! # Vì sao tách khỏi [`super::Router`]
//!
//! "Bao lâu được phép đề xuất một skill mới" là **chính sách cooldown + một khoá
//! chỗ**, không phải điều phối run. Trước đây nó nằm lọt trong `Router` dưới dạng
//! `learning_gate: Mutex<Option<DateTime>>` cộng hai hàm `reserve_learning` /
//! `release_learning`; phép tính `proposal_interval_minutes → Duration` bị **lặp
//! hai lần** với hai đoạn `unwrap_or(MAX)` giống hệt nhau.
//!
//! [`LearningGate`] gom trạng thái và chính sách vào một chỗ, đồng thời làm phép tính
//! cooldown chỉ tồn tại đúng một bản. Nhờ đó điều kiện "chờ tới hạn" được kiểm thử
//! được độc lập, không cần dựng cả `Router`.

use std::sync::Mutex;

use bean_skills::SkillCatalog;
use bean_types::Config;

/// Cổng cooldown cho đề xuất skill.
///
/// Sở hữu **chỗ đã giữ** của lần đề xuất đang bay; giữ chỗ (thay vì chỉ đọc mốc
/// thời gian) là để chống hai run kết thúc gần nhau cùng phát đề xuất.
#[derive(Debug, Default)]
pub(super) struct LearningGate {
    /// Mốc thời gian đã giữ chỗ; `None` ⇒ không có lượt nào đang chờ.
    reserved: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
}

impl LearningGate {
    /// Thử **giữ chỗ** cho một lượt reflection.
    ///
    /// Trả `None` khi: learning tắt, chưa có catalog, chưa tới hạn cooldown, hoặc
    /// state của catalog không đọc được. Trả `Some(mốc)` khi giữ chỗ thành công —
    /// mốc đó phải đưa lại cho [`Self::release`] nếu reflection không sinh được draft.
    pub(super) fn reserve(
        &self,
        config: &Config,
        catalog: Option<&SkillCatalog>,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        if !config.learning.enabled {
            return None;
        }
        let catalog = catalog?;
        let now = chrono::Utc::now();
        let cooldown = cooldown(config);
        match catalog.last_proposal_at() {
            // Đề xuất gần nhất của catalog chưa tới hạn ⇒ chờ.
            Ok(Some(last)) => {
                let Ok(last) = chrono::DateTime::parse_from_rfc3339(&last) else {
                    tracing::warn!("state learning có timestamp không hợp lệ");
                    return None;
                };
                if now < last.with_timezone(&chrono::Utc) + cooldown {
                    return None;
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(error = %error, "state learning không hợp lệ; bỏ qua reflection");
                return None;
            }
        }
        let mut reserved = self.reserved.lock().ok()?;
        // Còn lượt nào khác đang giữ chỗ trong cửa sổ cooldown ⇒ chờ nốt.
        if reserved.is_some_and(|last| now < last + cooldown) {
            return None;
        }
        *reserved = Some(now);
        Some(now)
    }

    /// Nhả chỗ khi reflection không tạo được draft.
    ///
    /// Chỉ nhả khi `reservation` **còn là** chỗ đang giữ — nếu không, một lượt mới đã
    /// giữ chỗ mới sẽ bị xoá nhầm và cooldown bị bỏ qua.
    pub(super) fn release(&self, reservation: chrono::DateTime<chrono::Utc>) {
        if let Ok(mut reserved) = self.reserved.lock()
            && *reserved == Some(reservation)
        {
            *reserved = None;
        }
    }
}

/// Chu kỳ chờ giữa hai lần đề xuất, lấy từ `learning.proposal_interval_minutes`.
///
/// Dùng `MAX` khi phép chuyển đổi tràn: cấu hình đã bị `Config::validate` chặn ở 0,
/// nên giá trị khổng lồ là hành vi fail-safe (chờ rất lâu) chứ không phải `unwrap`.
fn cooldown(config: &Config) -> chrono::Duration {
    chrono::Duration::from_std(std::time::Duration::from_secs(
        config.learning.proposal_interval_minutes.saturating_mul(60),
    ))
    .unwrap_or(chrono::Duration::MAX)
}
