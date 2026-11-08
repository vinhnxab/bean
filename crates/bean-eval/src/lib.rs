//! # bean-eval — khung đo chất lượng agent
//!
//! Đo **tác vụ có hoàn thành hay không** thay vì chỉ đo tính đúng đắn của code
//! (test đơn vị đã phủ phần đó). Mỗi task eval là một tác vụ thật: runner dựng
//! một môi trường **cách ly hoàn toàn** (thư mục tạm cho workspace + SQLite +
//! skills), chạy đúng một lượt qua [`bean_core::Router`] — cùng đường đi với CLI/web —
//! rồi chấm bộ khẳng định và ghi metrics.
//!
//! * Task khai báo bằng TOML (`evals/tasks/*.toml`): prompt, file setup, kịch bản
//!   `FakeProvider` (task xác định) hoặc cấu hình provider thật (task đo model
//!   thật, opt-in qua biến môi trường — không bao giờ đọc key từ repo).
//! * Metrics ghi JSONL một dòng mỗi task: kết quả, số bước, số tool call, token,
//!   số lần confirm, thời gian — đủ để so sánh trước/sau khi đổi prompt, tool
//!   description hay compaction thay vì đoán mò.
//! * Trajectory của mỗi run ghi JSON riêng để debug vì sao model chọn sai tool.
//!
//! Chạy: `bean eval run` (mặc định `evals/tasks`, ghi `evals/out`); xem lại:
//! `bean eval report <metrics.jsonl>`.
#![forbid(unsafe_code)]

mod assertion;
mod recorder;
mod report;
mod runner;
mod task;

pub use assertion::{Assertions, ExpectedEnd};
pub use recorder::UsageRecorder;
pub use report::{MetricsRecord, TaskSummary, print_report, summarize};
pub use runner::{ProviderChoice, RunOptions, TaskResult, run_task, run_tasks, run_tasks_in_dir};
pub use task::{EvalTask, PrefillSpec, TaskLlm, load_tasks_dir};
