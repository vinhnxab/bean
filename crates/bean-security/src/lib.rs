//! # bean-security
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

pub mod audit;
pub mod paths;
pub mod policy;
pub mod ratelimit;
pub mod sandbox;
pub mod shell;
pub mod ssrf;
pub mod untrusted;
pub mod web;

pub use audit::{AuditEntry, AuditLog, entry_now, redact_secrets, redact_text_secrets};
pub use paths::CapWorkspace;
pub use policy::{DenyReason, Policy, PolicyDecision, SessionPolicy, deny_list_reason};
pub use ratelimit::{DEFAULT_MAX_LOCK, DEFAULT_THRESHOLD, DEFAULT_WINDOW, RateLimiter};
pub use sandbox::{Sandbox, SandboxError, ShellOutcome};
pub use shell::{run_shell, run_shell_for_projects};
pub use ssrf::{FetchedPage, SafeHttpClient, SsrfError};
pub use untrusted::{UntrustedFlag, wrap as wrap_untrusted};
pub use web::{SearchConfigError, web_fetch, web_search};
