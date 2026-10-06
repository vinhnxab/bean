//! Ngân sách token: bốn hàm công khai đặt trên trait [`Store`](super::Store).
//!
//! # Vì sao tách khỏi [`super`]
//!
//! "Còn bao nhiêu token hôm nay" là **chính sách phụ thuộc vào store nhưng thuộc về
//! tầng trên** (agent loop và scheduler gọi, không ai trong store gọi lại). Trước đây
//! chúng nằm giữa `trait Store` và bản in-memory, khiến đọc `trait` phải dừng lại ở
//! giữa một loạt hàm không liên quan.
//!
//! Giữ chúng ở đây cũng gom được **nguyên tắc chung**: cả bốn hàm đều là
//! "ghi rồi đọc lại" (xem [`record_usage`]) — đó là điểm cần đọc một lần duy nhất,
//! thay vì rải bốn bản triển khai gần như giống nhau ở bốn chỗ.

use bean_types::Usage;

use super::{Store, StoreError};
/// Kiểm tra ngân sách trước một lượt gọi LLM mới.
///
/// `Usage` là số token thực trả về bởi provider. Khi đã chạm trần, caller phải dừng và
/// báo người dùng; lượt gọi mới không được gửi đi. Ngày được truyền rõ ràng để test và
/// scheduler có thể kiểm soát mốc UTC.
pub async fn ensure_daily_budget(
    store: &dyn Store,
    day: &str,
    limit: u64,
) -> Result<(), StoreError> {
    let usage = store.usage(day).await?;
    let used = u64::from(usage.total());
    if used >= limit {
        return Err(StoreError::BudgetExceeded { used, limit });
    }
    Ok(())
}

/// Ghi usage và trả tổng mới của ngày.
///
/// Việc đọc lại sau `add_usage` giữ API store đơn giản cho cả SQLite và MemoryStore;
/// lớp gọi LLM chỉ cần một cổng cập nhật usage duy nhất.
pub async fn record_usage(store: &dyn Store, day: &str, usage: Usage) -> Result<Usage, StoreError> {
    store.add_usage(day, usage).await?;
    store.usage(day).await
}

/// Kiểm tra ngân sách token **riêng của một role** trước lượt gọi LLM mới (M21.7).
///
/// Tách khỏi [`ensure_daily_budget`] (tổng toàn instance) để Developer chạy vòng lặp dài không
/// ăn hết hạn mức khiến Monitor/Security-scan không chạy được job định kỳ.
pub async fn ensure_role_daily_budget(
    store: &dyn Store,
    day: &str,
    role: &str,
    limit: u64,
) -> Result<(), StoreError> {
    let usage = store.usage_by_role(day, role).await?;
    let used = u64::from(usage.total());
    if used >= limit {
        return Err(StoreError::BudgetExceeded { used, limit });
    }
    Ok(())
}

/// Ghi usage vừa lượt gọi vào **hai** sổ: tổng (`/api/status`) và riêng role (M21.7).
pub async fn record_usage_for_role(
    store: &dyn Store,
    day: &str,
    role: &str,
    usage: Usage,
) -> Result<Usage, StoreError> {
    store.add_usage_by_role(day, role, usage).await?;
    store.usage_by_role(day, role).await
}
