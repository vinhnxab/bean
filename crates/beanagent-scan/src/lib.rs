//! # BeanAgent-scan
//!
//! Domain **quét bảo mật** (Plan.md M23), read-only về phía hệ thống được quét.
//!
//! Ba bất biến — đọc theo thứ tự tầng, tầng ngoài phải đúng trước khi tầng trong chạy:
//!
//! 1. **Target phải nằm trong `[[infra_scope]]`** — kiểm ở [`scope::ScanScope`], tầng code,
//!    **trước** khi spawn scanner. `Plan.md` M23 nói rõ *"kiểm ở tầng code, không dựa vào
//!    model tự kiểm tra"*. Scope rỗng ⇒ từ chối mọi thứ (fail-closed).
//! 2. **Model không bao giờ chạm vào argv** — tool chỉ nhận `target` + `ports`, tự kiểm định dạng
//!    rồi **tự dựng** argv của scanner. Không có `sh -c` ở đường này, nên `;` hay `&&` trong
//!    tham số chỉ là ký tự vô nghĩa chứ không phải lệnh (D14.5).
//! 3. **Luôn hỏi xác nhận, không có "cho phép trong phiên"** — khai báo
//!    [`Risk::Dangerous`], và `Policy::decide` tự trả `allow_in_session: false` cho mức này
//!    (mục 7.2). Không cần (và không được) thêm logic riêng.
//!
//! Ngoài ra: output scanner được bọc `<untrusted_content>` (mục 15.4) vì banner mà scanner
//! đọc được từ target là **dữ liệu kẻ tấn công kiểm soát được** — đúng loại payload mà S1
//! (2026-09-26) đã vá cho `read_file`/`run_shell`.
#![forbid(unsafe_code)]

mod scope;
mod tool;

pub use scope::ScanScope;
pub use tool::{ScanReport, ScannerCmd, security_scan};

// Re-export để test và adapter dùng chung đúng một kiểu cảnh báo (M23).
pub use beanagent_types::config::INFRA_SCAN_TAG;
