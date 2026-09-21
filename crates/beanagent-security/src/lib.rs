//! # BeanAgent-security
//!
//! Path jail, sandbox shell, SSRF, policy/confirm, audit log (agents.md mục 15).
//!
//! * **M4**: `paths` (cap-std `Dir` gốc workspace, chặn `..`, đường dẫn tuyệt đối và symlink
//!   thoát ra — test bằng `proptest`), `sandbox` (`docker run --rm`, chỉ mount workspace,
//!   user non-root, `--network none`, `--cap-drop ALL`, no-new-privileges, container có tên để
//!   `docker kill` khi timeout), `policy` (Safe/Confirm/Dangerous, "cho phép trong phiên",
//!   deny-list chỉ là lớp phụ, audit JSONL có redact secret), và hàm bọc `<untrusted_content>`.
//! * **M7**: `ssrf` (resolver DNS tuỳ biến cho `reqwest::ClientBuilder::dns_resolver()` —
//!   lọc IP **ngay lúc kết nối** để chống DNS rebinding; chặn private/loopback/link-local/
//!   metadata `169.254.169.254` và IPv6 tương ứng; kiểm tra lại từng bước redirect).
#![forbid(unsafe_code)]
