//! # bean-marketing
//!
//! Domain **marketing** (Plan.md M24), tách biệt hoàn toàn khỏi infra/dev/finance.
//!
//! Ba công cụ, mỗi cái một mức rủi ro khác nhau — cố ý **không** gộp:
//!
//! 1. `marketing_draft` (`marketing-draft`) — chỉ **ghi file** vào workspace, tuyệt đối
//!    không gọi mạng. Đây là bước mặc định, ai cũng chạy được trong lúc làm việc.
//! 2. `marketing_publish` (`marketing-publish`) — đăng thật ra ngoài. **Luôn
//!    `Dangerous`**, khai cứng trong code chứ không đọc từ cấu hình, nên "cho phép trong
//!    phiên" không bao giờ khả dụng (mục 7.2) — đăng bài là hành động **không hoàn tác**.
//! 3. Đọc web: **không** tạo tool mới, dùng lại `web_fetch`/`web_search` sẵn có và mở
//!    thêm tag `marketing-read` (xem [`bean_tools::Tool::also_visible_to`]).
//!
//! Vì sao không tạo tool đăng riêng cho từng nền tảng: endpoint + credential lấy từ cấu hình
//! (như M22a) ⇒ đổi nền tảng chỉ sửi `bean.toml`, không phải build lại binary.
#![forbid(unsafe_code)]

mod draft;
mod publish;

pub use draft::{MarketingDraftParams, marketing_draft};
pub use publish::{MarketingPublishParams, PublishClient, PublishError, marketing_publish};
