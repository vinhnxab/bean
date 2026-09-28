//! # bean-qa
//!
//! Domain **chạy test suite** cho vai trò `qa` (M27), read-only về phía dự án được test.
//!
//! Đóng đúng khoảng trống: tag `test-run` đã tồn tại trong RBAC (M21) nhưng **chưa tool nào
//! dùng**, nên role `qa` không chạy được lệnh test nào — dù `Plan.md` mục 2b giao cho nó
//! đúng việc *"review diff, chạy test độc lập"*. `run_shell` không thay thế được vì nó
//! yêu cầu `dev-write`/`infra-scan`, tức là mở `run_shell` cho `qa` là phá four-eyes.
//!
//! Bốn bất biến — đọc theo thứ tự tầng, tầng ngoài phải đúng trước khi tầng trong chạy:
//!
//! 1. **Suite phải nằm trong `[[qa.suites]]`** — kiểm ở [`suite::SuiteCatalog`], tầng
//!    code, **trước** khi spawn runner. `[[qa.suites]]` rỗng ⇒ từ chối mọi thứ
//!    (fail-closed, y hệt D14.1 của M23).
//! 2. **Model không bao giờ chạm vào argv** — tool chỉ nhận `suite_name` + `filter`, mọi
//!    argv do [`suite::build_argv`] sinh trong code. Không có `sh -c` ở đường này
//!    (`Sandbox::run_argv_readonly` exec thẳng), nên `;` hay `&&` trong `filter` chỉ là
//!    ký tự vô nghĩa chứ không phải lệnh (D14.5).
//! 3. **Không gì được ghi vào workspace dự án** — bảo đảm ở tầng mount (`:ro`) của
//!    [`bean_security::Sandbox::run_argv_readonly`], không phải ở lời hứa. Kể cả
//!    `build.rs` hay test tự ghi cũng không chạm được vào cây thư mục dự án.
//! 4. **Mức rủi ro `Confirm`, KHÔNG `Dangerous`** — đọc kết quả test không phải quét hạ
//!    tầng. Khác `security_scan` của M23, `qa_test` **có** tuỳ chọn "cho phép trong
//!    phiên"; điều kiện ràng buộc quyết định đó là bất biến 3 phải giữ nguyên
//!    (xem `docs/decisions.md` D17.3).
//!
//! Ngoài ra: `QaReport` **và** raw log đều bọc `<untrusted_content>` (mục 15.4) vì tên
//! test/fixture/assertion message nằm trong quyền kiểm soát của code dự án — đúng loại
//! nguồn dữ liệu mà S1 (2026-09-26) đã vá cho `read_file`/`run_shell`. Không có ngoại lệ,
//! kể cả với trường đã "chuẩn hoá".
#![forbid(unsafe_code)]

mod report;
mod suite;
mod tool;

pub use report::{FailedTest, QaReport, QaStatus, parse_output};
pub use suite::{SuiteCatalog, build_argv, sanitize_filter};
pub use tool::{QaTestParams, qa_test};

// Re-export để test và adapter dùng chung đúng một hằng tag RBAC (M27).
pub use bean_types::config::TEST_RUN_TAG;
