//! `QaReport` — báo cáo chuẩn hoá mà Manager đọc (M27, mục 5).
//!
//! # Vì sao KHÔNG trả raw log cho Manager
//!
//! `Plan.md` mục 3 nói rõ: *"Manager không đọc dữ liệu thô của agent con — chỉ đọc báo cáo
//! đã chuẩn hoá"*. Raw log của `cargo test` có hàng nghìn dòng compile, còn log pytest/vitest
//! nhúng cả stack trace và source code của dự án. Hình dạng ở đây **cố ý giống hệt**
//! `ScanReport` của M23 để Manager đọc mọi agent con theo một kiểu.
//!
//! # Vì sao cả report lẫn raw log đều bọc untrusted
//!
//! Tên test, tên fixture, assertion message đều nằm trong **quyền kiểm soát của code dự
//! án** — đúng loại nguồn mà S1 (2026-09-26) đã vá cho `read_file`/`run_shell` (mục 22.5).
//! Không có ngoại lệ cho trường "đã chuẩn hoá": việc chuẩn hoá chỉ giảm khối lượng, không
//! làm nội dung đáng tin hơn.
//!
//! # Định dạng thật, không đoán
//!
//! Ba parser dưới đây viết theo output thật đã chạy thử, không suy đoán từ tài liệu:
//!
//! * `cargo test` — `test result: FAILED. 2 passed; 1 failed; 0 ignored; …` và danh sách
//!   `    <tên test>` trong khối `failures:` cuối.
//! * `pytest` — `FAILED test_demo.py::test_bad - AssertionError: …` (dòng ngay trước dòng
//!   tổng kết `1 failed, 1 passed in 0.03s`).
//! * `vitest` — `      Tests  1 failed | 1 passed (2)` và các dòng ` FAIL  <file> > <tên>`.

use beanagent_types::config::QaRunner;
use serde::Serialize;

/// Trần số test fail đưa vào báo cáo (Manager không cần hàng trăm dòng).
const MAX_FAILED_TESTS: usize = 50;

/// Trần ký tự cho `FailedTest::message`.
const MAX_MESSAGE_CHARS: usize = 300;

/// Kết quả tổng quát của một lần chạy suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QaStatus {
    /// Tất cả test đều pass (có thể có test bị bỏ qua — `ignored` không phải fail).
    Passed,
    /// Runner chạy xong và có test fail.
    Failed,
    /// Runner **không chạy được** (build hỏng, thiếu lệnh, timeout, container chết).
    ///
    /// Tách khỏi `Failed` vì hai thứ này khác hẳn về mặt nghiệp vụ: `Failed` là dự án hỏng,
    /// còn `Error` là **hạ tầng kiểm thử** hỏng — không thể kết luận gì về chất lượng code.
    Error,
}

/// Một test fail, rút gọn (không phải full stack trace).
#[derive(Debug, Clone, Serialize)]
pub struct FailedTest {
    /// Tên test đầy đủ (vd `core::tools::test_abc`, `tests/test_api.py::test_login`).
    pub name: String,
    /// Thông điệp assertion, đã cắt còn tối đa 300 ký tự.
    pub message: String,
}

/// Báo cáo chuẩn hoá mà Manager đọc — cùng hình dạng với `ScanReport` của M23.
#[derive(Debug, Clone, Serialize)]
pub struct QaReport {
    /// Trạng thái tổng quát.
    pub status: QaStatus,
    /// Tóm tắt một dòng, vd `12 passed, 2 failed, 0 skipped (suite: core-unit)`.
    pub summary: String,
    /// Danh sách rủi ro — ở đây là tên test fail rút gọn.
    pub risks: Vec<String>,
    /// Chi tiết test fail (đã cắt số lượng và độ dài message).
    pub failed_tests: Vec<FailedTest>,
}

impl QaReport {
    /// Báo cáo cho trường hợp **từ chối ở tầng code** (suite rỗng / không có trong catalog).
    ///
    /// Vẫn là `Error` chứ không phải `Failed`: không hề có test nào chạy.
    #[must_use]
    pub fn refused(reason: String) -> Self {
        Self {
            status: QaStatus::Error,
            summary: reason.clone(),
            risks: Vec::new(),
            failed_tests: Vec::new(),
        }
    }

    /// Báo cáo cho trường hợp runner **không chạy được**.
    #[must_use]
    pub fn runner_error(suite: &str, reason: String) -> Self {
        Self {
            status: QaStatus::Error,
            summary: format!("không chạy được suite `{suite}`: {reason}"),
            risks: vec![reason],
            failed_tests: Vec::new(),
        }
    }
}

/// Cắt message ở **ranh giới UTF-8** (mục 22.9) — cắt byte sẽ panic với tiếng Việt/emoji.
fn cap_message(text: &str) -> String {
    match beanagent_tools::truncate_chars(text.trim(), MAX_MESSAGE_CHARS) {
        Some((kept, cut)) => format!("{kept}… [cắt {cut} ký tự]"),
        None => text.trim().to_string(),
    }
}

/// Dựng `QaReport` từ số đếm + danh sách test fail.
fn build(
    suite: &str,
    passed: u32,
    failed: u32,
    skipped: u32,
    failed_tests: Vec<FailedTest>,
) -> QaReport {
    let truncated = failed_tests.len() > MAX_FAILED_TESTS;
    let mut failed_tests = failed_tests;
    failed_tests.truncate(MAX_FAILED_TESTS);
    let risks: Vec<String> = failed_tests
        .iter()
        .map(|t| t.name.clone())
        .chain(truncated.then(|| format!("(còn {} test fail nữa)", MAX_FAILED_TESTS)))
        .collect();
    QaReport {
        status: if failed > 0 {
            QaStatus::Failed
        } else {
            QaStatus::Passed
        },
        summary: format!("{passed} passed, {failed} failed, {skipped} skipped (suite: {suite})"),
        risks,
        failed_tests,
    }
}

/// Parse output thật của runner, ghép stdout + stderr (pytest/vitest ghi từng phần khác nhau).
///
/// `exit_code` = `None` nghĩa là tiến trình bị kill (timeout) — không phải test fail.
#[must_use]
pub fn parse_output(
    runner: QaRunner,
    suite: &str,
    exit_code: Option<i32>,
    output: &str,
) -> QaReport {
    match runner {
        QaRunner::CargoTest => parse_cargo(suite, exit_code, output),
        QaRunner::Vitest => parse_vitest(suite, exit_code, output),
        QaRunner::Pytest => parse_pytest(suite, exit_code, output),
    }
}

/// Đọc số **đứng trước** một nhãn trong dòng tổng kết; `0` nếu không có nhãn đó.
///
/// Dùng `rsplit_once`: số nằm **trước** nhãn (`2 passed` ⇒ số là `2`), nên phải tách từ
/// phải sang trái — `split_once` sẽ tìm nhãn đầu tiên rồi cắt mất chính con số cần đọc.
fn count_before(text: &str, label: &str) -> u32 {
    text.rsplit_once(label)
        .and_then(|(head, _)| head.rsplit(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
}

/// Dòng cuối có nội dung — dùng làm thông điệp khi runner chết giữa chừng.
fn last_meaningful_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        // `filter(..).next_back()` không hợp lệ vì `Filter` không phải `DoubleEndedIterator`
        // → dùng `rfind`, vẫn là dòng **cuối** có nội dung.
        .rfind(|line| !line.is_empty())
        .map(cap_message)
}

/// `cargo test` — định dạng thật:
/// ```text
/// test also_ok_two ... ok
/// test fails_on_purpose ... FAILED
/// failures:
///     fails_on_purpose
/// test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; …
/// ```
///
/// `cargo test --workspace` in ra **nhiều** dòng `test result:` (mỗi target một dòng) nên
/// phải **cộng dồn**, không lấy dòng cuối.
fn parse_cargo(suite: &str, exit_code: Option<i32>, output: &str) -> QaReport {
    let (mut passed, mut failed, mut skipped) = (0u32, 0u32, 0u32);
    let mut saw_result = false;
    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("test result:") else {
            continue;
        };
        saw_result = true;
        passed += count_before(rest, " passed;");
        failed += count_before(rest, " failed;");
        skipped += count_before(rest, " ignored;");
    }
    if !saw_result {
        // Không có dòng `test result:` ⇒ build hỏng / runner chết, không phải test fail.
        return QaReport::runner_error(
            suite,
            last_meaningful_line(output)
                .unwrap_or_else(|| "runner không in dòng kết quả nào".to_string()),
        );
    }
    // Exit code khác 0 mà không có test fail nào = build hỏng, không phải test hỏng.
    if failed == 0 && exit_code.is_some_and(|code| code != 0) {
        return QaReport::runner_error(suite, "build lỗi, test chưa chạy được".to_string());
    }
    // Tên test fail nằm trong khối `failures:` gần nhất (4 space indent + tên).
    let mut failed_tests = Vec::new();
    let mut in_failures = false;
    for line in output.lines() {
        if line.trim_end() == "failures:" {
            in_failures = true;
            continue;
        }
        if line.starts_with("test result:") {
            in_failures = false;
        }
        if !in_failures {
            continue;
        }
        let Some(name) = line.strip_prefix("    ") else {
            continue;
        };
        let name = name.trim();
        // `---- fails_on_purpose stdout ----` là tiêu đề khối log, không phải tên test.
        if name.is_empty() || name.contains(" stdout ") {
            continue;
        }
        failed_tests.push(FailedTest {
            name: name.to_string(),
            message: String::new(),
        });
    }
    build(suite, passed, failed, skipped, failed_tests)
}

/// `pytest` — định dạng thật (đã chạy thật với pytest trong `python:3.12-slim`):
/// ```text
/// FAILED test_demo.py::test_bad - AssertionError: so sanh sai
/// 1 failed, 1 passed in 0.03s
/// ```
fn parse_pytest(suite: &str, exit_code: Option<i32>, output: &str) -> QaReport {
    let mut failed_tests = Vec::new();
    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("FAILED ") else {
            continue;
        };
        // `path::name - message` (message có thể vắng mặt).
        let (name, message) = match rest.split_once(" - ") {
            Some((name, msg)) => (name.trim(), cap_message(msg)),
            None => (rest.trim(), String::new()),
        };
        failed_tests.push(FailedTest {
            name: name.to_string(),
            message,
        });
    }
    let Some(summary) = output.lines().rev().find(|line| {
        line.contains(" passed") || line.contains(" failed") || line.contains(" error")
    }) else {
        return QaReport::runner_error(
            suite,
            last_meaningful_line(output)
                .unwrap_or_else(|| "pytest không in dòng tổng kết".to_string()),
        );
    };
    let passed = count_before(summary, " passed");
    let failed = count_before(summary, " failed");
    let skipped = count_before(summary, " skipped") + count_before(summary, " deselected");
    if failed == 0 && exit_code.is_some_and(|code| code != 0) {
        return QaReport::runner_error(suite, "pytest lỗi khi thu thập test".to_string());
    }
    build(suite, passed, failed, skipped, failed_tests)
}

/// `vitest run` — định dạng thật (đã chạy thật với vitest 5.0.1):
/// ```text
///  FAIL  src/__fmtprobe.test.ts > probe > fails_on_purpose
/// AssertionError: expected 4 to be 5
///  Test Files  1 failed (1)
///       Tests  1 failed | 1 passed (2)
/// ```
fn parse_vitest(suite: &str, exit_code: Option<i32>, output: &str) -> QaReport {
    // vitest in tên test ở dòng `FAIL  <file> > <suite> > <tên>` và **message ở dòng kế
    // tiếp** (thường là `AssertionError: ...`) — nên phải đọc theo cặp dòng, không phải
    // theo từng dòng độc lập.
    let mut failed_tests = Vec::new();
    let mut pending_name: Option<String> = None;
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("FAIL ") {
            pending_name = Some(rest.trim().to_string());
            continue;
        }
        let Some(name) = pending_name.take() else {
            continue;
        };
        // Dòng ngay sau `FAIL` là message; dòng nào rỗng cũng bỏ qua, và chỉ nhận dòng
        // có thông tin (không phải khối trang trí `───` hay dòng kế tiếp của danh sách).
        if trimmed.is_empty() || trimmed.starts_with('─') || trimmed.starts_with('⎯') {
            continue;
        }
        let message = trimmed
            .strip_prefix("AssertionError:")
            .map_or_else(|| cap_message(trimmed), cap_message);
        failed_tests.push(FailedTest { name, message });
    }
    let totals = output
        .lines()
        .find(|line| line.trim_start().starts_with("Tests "));
    let Some(totals) = totals else {
        return QaReport::runner_error(
            suite,
            last_meaningful_line(output)
                .unwrap_or_else(|| "vitest không in dòng tổng kết".to_string()),
        );
    };
    // `Tests  1 failed | 1 passed | 2 skipped (5)` — vitest dùng `|` làm dấu phân tách.
    let (mut passed, mut failed, mut skipped) = (0u32, 0u32, 0u32);
    for part in totals.split('|') {
        let Some(number) = part.split_whitespace().find_map(|w| w.parse::<u32>().ok()) else {
            continue;
        };
        if part.contains("failed") {
            failed += number;
        } else if part.contains("passed") {
            passed += number;
        } else if part.contains("skipped") || part.contains("todo") {
            skipped += number;
        }
    }
    if failed == 0 && exit_code.is_some_and(|code| code != 0) {
        return QaReport::runner_error(suite, "vitest lỗi khi thu thập test".to_string());
    }
    build(suite, passed, failed, skipped, failed_tests)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{QaStatus, parse_output};
    use beanagent_types::config::QaRunner;

    /// Output thật của `cargo test` (đã chạy thật: 2 pass, 1 fail cố ý).
    const CARGO_FAIL: &str = "\
running 3 tests
test also_ok_two ... ok
test fails_on_purpose ... FAILED
test ok_one ... ok

failures:

---- fails_on_purpose stdout ----

thread 'fails_on_purpose' panicked at src/lib.rs:8:25:
assertion `left == right` failed: so sanh sai

failures:
    fails_on_purpose

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";

    /// `cargo test --workspace` in nhiều dòng `test result:` — phải cộng dồn, không lấy
    /// dòng cuối (lấy dòng cuối thì báo cáo sai số test).
    #[test]
    fn cargo_sums_every_test_result_line() {
        let output = "\
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.2s
";
        let report = parse_output(QaRunner::CargoTest, "core", Some(0), output);
        assert_eq!(report.status, QaStatus::Passed);
        assert!(report.summary.contains("15 passed"), "{}", report.summary);
        assert!(report.summary.contains("0 failed"), "{}", report.summary);
    }

    /// Không panic khi có test fail, và phản ánh đúng số + tên test fail.
    #[test]
    fn cargo_reports_failed_test_without_panicking() {
        let report = parse_output(QaRunner::CargoTest, "core-unit", Some(101), CARGO_FAIL);
        assert_eq!(report.status, QaStatus::Failed);
        assert!(
            report.summary.contains("2 passed") && report.summary.contains("1 failed"),
            "{}",
            report.summary
        );
        assert!(
            report.summary.contains("suite: core-unit"),
            "{}",
            report.summary
        );
        assert_eq!(report.failed_tests.len(), 1, "{:?}", report.failed_tests);
        assert_eq!(report.failed_tests[0].name, "fails_on_purpose");
        assert_eq!(report.risks, vec!["fails_on_purpose".to_string()]);
        // Tiêu đề `---- ... stdout ----` không được nhầm là tên test.
        assert!(!report.failed_tests[0].name.contains("stdout"));
    }

    #[test]
    fn cargo_handles_zero_tests_run() {
        let output = "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished\n";
        let report = parse_output(QaRunner::CargoTest, "core", Some(0), output);
        assert_eq!(report.status, QaStatus::Passed);
        assert!(report.failed_tests.is_empty());
    }

    /// Build hỏng (không có dòng `test result:`) ⇒ `Error`, KHÔNG phải `Failed` — hai thứ
    /// này khác hẳn về nghiệp vụ: `Failed` là dự án hỏng, `Error` là hạ tầng test hỏng.
    #[test]
    fn cargo_build_failure_is_error_not_failed() {
        let output =
            "error[E0428]: could not compile `probe`\nerror: test failed, to rerun pass `--lib`\n";
        let report = parse_output(QaRunner::CargoTest, "core", Some(101), output);
        assert_eq!(report.status, QaStatus::Error);
        assert!(!report.summary.contains("passed"), "{}", report.summary);
    }

    /// Tiến trình bị kill (`exit_code = None`, timeout) phải là `Error`, không phải `Failed`.
    #[test]
    fn killed_runner_is_error_not_failed() {
        let report = parse_output(QaRunner::CargoTest, "core", None, "");
        assert_eq!(report.status, QaStatus::Error);
    }

    /// Output thật của `pytest` (đã chạy thật trong `python:3.12-slim`).
    const PYTEST_FAIL: &str = "\
.F                                                                       [100%]
=================================== FAILURES ===================================
___________________________________ test_bad ___________________________________

    def test_bad():
>       assert x == 4, \"so sanh sai\"
E       AssertionError: so sanh sai

=========================== short test summary info ============================
FAILED test_demo.py::test_bad - AssertionError: so sanh sai
1 failed, 1 passed in 0.03s
";

    #[test]
    fn pytest_reports_failed_test_with_message() {
        let report = parse_output(QaRunner::Pytest, "api", Some(1), PYTEST_FAIL);
        assert_eq!(report.status, QaStatus::Failed);
        assert!(report.summary.contains("1 failed"), "{}", report.summary);
        assert!(report.summary.contains("1 passed"), "{}", report.summary);
        assert_eq!(report.failed_tests.len(), 1, "{:?}", report.failed_tests);
        assert_eq!(report.failed_tests[0].name, "test_demo.py::test_bad");
        assert!(
            report.failed_tests[0].message.contains("so sanh sai"),
            "{:?}",
            report.failed_tests[0].message
        );
    }

    /// Output thật của `vitest run` (đã chạy thật với vitest 5.0.1).
    const VITEST_FAIL: &str = "\
 RUN  v5.0.1 /w

 ❯ src/__fmtprobe.test.ts (2 tests | 1 failed)54ms
   ❯ probe (2)
     ✓ passes 7ms
     × fails_on_purpose 20ms

⎯⎯⎯⎯⎯⎯ Failed Tests 1 ⎯⎯⎯⎯⎯⎯⎯

 FAIL  src/__fmtprobe.test.ts > probe > fails_on_purpose
AssertionError: expected 4 to be 5 // Object.is equality

 Test Files  1 failed (1)
      Tests  1 failed | 1 passed (2)
";

    #[test]
    fn vitest_reports_failed_test_with_message() {
        let report = parse_output(QaRunner::Vitest, "web", Some(1), VITEST_FAIL);
        assert_eq!(report.status, QaStatus::Failed);
        assert!(report.summary.contains("1 failed"), "{}", report.summary);
        assert!(report.summary.contains("1 passed"), "{}", report.summary);
        assert_eq!(report.failed_tests.len(), 1, "{:?}", report.failed_tests);
        assert!(
            report.failed_tests[0].name.contains("fails_on_purpose"),
            "{:?}",
            report.failed_tests[0].name
        );
        assert!(
            report.failed_tests[0]
                .message
                .contains("expected 4 to be 5"),
            "{:?}",
            report.failed_tests[0].message
        );
    }

    #[test]
    fn vitest_all_passed_is_passed_status() {
        let output = "\
 ✓ src/i18n/i18n.test.ts (3 tests) 34ms

 Test Files  1 passed (1)
      Tests  3 passed (3)
";
        let report = parse_output(QaRunner::Vitest, "web", Some(0), output);
        assert_eq!(report.status, QaStatus::Passed);
        assert!(report.summary.contains("3 passed"), "{}", report.summary);
        assert!(report.failed_tests.is_empty());
    }
}
