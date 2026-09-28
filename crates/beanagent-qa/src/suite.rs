//! `SuiteCatalog` — tra cứu suite đã khai báo + dựng argv (M27).
//!
//! # Vì sao đây là thứ quan trọng nhất của M27
//!
//! Agent có quyền chạy lệnh (qua `run_shell`, tag `dev-write`). Nếu model — bị prompt
//! injection dẫn dắt — tự chọn được lệnh cần chạy thì vai trò `qa` biến thành đường chạy
//! lệnh tuỳ ý, đúng thứ mà `forbid_tags = ["dev-write"]` của role `qa` cấm. Vì vậy mọi
//! lần gọi đều phải tra cứu suite ở **tầng code** trước khi spawn, và **không** có
//! đường nào nhận lệnh tự do — cùng tinh thần D14.1/D14.5 của M23.
//!
//! # Vì sao `filter` vẫn phải làm sạch dù đã exec thẳng
//!
//! [`beanagent_security::Sandbox::run_argv_readonly`] không có `sh -c`, nên ký tự shell
//! trong `filter` **không thể** trở thành lệnh. Nhưng `filter` là chuỗi của *model* đi vào
//! argv và vào báo cáo, nên vẫn phải làm sạch ở ranh giới: giới hạn độ dài, cấm ký tự
//! điều khiển, cấm ký tự shell. Đây là lớp phụ cạnh rào cản chính là "exec thẳng" —
//! đúng như deny-list mẫu so với sandbox (mục 15.3).

use std::collections::BTreeMap;

use beanagent_types::config::{QaRunner, QaSuiteConfig};

/// Trần độ dài `filter` (ký tự). Tên test dài hơn thế gần như không tồn tại.
const MAX_FILTER_CHARS: usize = 200;

/// Bộ ký tự được phép trong `filter`.
///
/// Cố ý **hẹp hơn nhiều** so với "ký tự không phải shell metachar": chỉ chữ, số và vài
/// ký tự hay gặp trong tên test (`_ - . : /`). Bất kỳ ký tự nào khác kể cả khoảng trắng,
/// `;`, `|`, `$`, backtick, quote, `\` đều bị từ chối — không phải vì chắc chắn là tấn
/// công, mà vì một `filter` hợp lệ không bao giờ cần tới chúng.
fn is_allowed_filter_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/')
}

/// Làm sạch `filter` do model gửi, hoặc trả lý do từ chối.
///
/// Hàm này **không** cố "escape" ký tự nguy hiểm: escape trong ngữ cảnh không có shell thì
/// chỉ làm hỏng tên test. Thay vào đó từ chối kèm thông điệp rõ để model tự sửa (mục 6).
pub fn sanitize_filter(raw: &str) -> Result<Option<String>, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > MAX_FILTER_CHARS {
        return Err(format!(
            "filter dài quá {MAX_FILTER_CHARS} ký tự — chỉ cần tên hoặc mẫu tên test"
        ));
    }
    if let Some(bad) = value.chars().find(|c| !is_allowed_filter_char(*c)) {
        return Err(format!(
            "filter chứa ký tự không hợp lệ `{bad}` — chỉ chấp nhận chữ, số và `_ - . : /` \
             (không nhận khoảng trắng hay ký tự shell)"
        ));
    }
    Ok(Some(value.to_string()))
}

/// Dựng argv **đầy đủ** cho một lần chạy suite, không qua shell (D14.5).
///
/// `fixed` là `args` khai trong `[[qa.suites]]` (cấu hình của người quản trị), `filter` là
/// tham số của model đã qua [`sanitize_filter`]. `filter` của `cargo_test` được đặt sau
/// `--` để không bao giờ bị parser hiểu nhầm thành tuỳ chọn khác.
#[must_use]
pub fn build_argv(runner: QaRunner, fixed: &[String], filter: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = Vec::with_capacity(fixed.len() + 5);
    match runner {
        QaRunner::CargoTest => {
            argv.push("cargo".into());
            argv.push("test".into());
            argv.extend(fixed.iter().cloned());
            if let Some(filter) = filter {
                argv.push("--".into());
                argv.push(filter.to_string());
            }
        }
        QaRunner::Vitest => {
            argv.push("pnpm".into());
            argv.push("exec".into());
            argv.push("vitest".into());
            argv.push("run".into());
            argv.extend(fixed.iter().cloned());
            if let Some(filter) = filter {
                argv.push(filter.to_string());
            }
        }
        QaRunner::Pytest => {
            argv.push("python".into());
            argv.push("-m".into());
            argv.push("pytest".into());
            argv.extend(fixed.iter().cloned());
            if let Some(filter) = filter {
                argv.push("-k".into());
                argv.push(filter.to_string());
            }
        }
    }
    argv
}

/// Một suite đã nạp từ cấu hình, sẵn sàng chạy.
#[derive(Debug, Clone)]
pub struct QaSuite {
    /// Tên suite (model chỉ được chọn đúng giá trị này).
    pub name: String,
    /// Loại runner — quyết định cách dựng argv.
    pub runner: QaRunner,
    /// Thư mục chạy, tương đối so với workspace (rỗng = gốc).
    pub workdir: String,
    /// Tham số cố định của runner.
    pub fixed_args: Vec<String>,
}

/// Danh sách suite được phép chạy, tra theo tên.
///
/// **Rỗng = từ chối mọi thứ** (fail-closed) — y hệt `ScanScope` rỗng của M23. Bật `[qa]`
/// mà chưa khai suite thì tool tồn tại nhưng không chạy được gì, thay vì chạy bất cứ thứ gì.
#[derive(Debug, Clone, Default)]
pub struct SuiteCatalog {
    suites: BTreeMap<String, QaSuite>,
}

impl SuiteCatalog {
    /// Nạp từ `[[qa.suites]]`; entry trùng tên bị bỏ qua (đã có `validate_qa()` chặn).
    #[must_use]
    pub fn from_config(entries: &[QaSuiteConfig]) -> Self {
        let mut suites = BTreeMap::new();
        for entry in entries {
            let name = entry.name.trim().to_string();
            if name.is_empty() {
                continue;
            }
            // `workdir` rỗng = gốc workspace. `SuiteCatalog` chuẩn hoá thành `.` ngay tại
            // đây (đúng một chỗ) vì `run_argv_readonly` ghép thành `/workspace/{workdir}` —
            // chuỗi rỗng sẽ cho `--workdir ""` và bị docker từ chối với lỗi khó hiểu.
            let workdir = match entry.workdir.trim() {
                "" => ".".to_string(),
                other => other.to_string(),
            };
            suites.insert(
                name.clone(),
                QaSuite {
                    name,
                    runner: entry.runner,
                    workdir,
                    fixed_args: entry.args.clone(),
                },
            );
        }
        Self { suites }
    }

    /// Catalog có rỗng không? (rỗng ⇒ mọi lần gọi đều bị từ chối)
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.suites.is_empty()
    }

    /// Tra suite theo tên; `None` nếu chưa khai báo hoặc catalog rỗng.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&QaSuite> {
        self.suites.get(name.trim())
    }

    /// Danh sách tên suite — dùng để dựng mô tả tool cho model.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.suites.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{MAX_FILTER_CHARS, SuiteCatalog, build_argv, sanitize_filter};
    use beanagent_types::config::{QaRunner, QaSuiteConfig};

    fn cfg(name: &str, runner: QaRunner, args: &[&str]) -> QaSuiteConfig {
        QaSuiteConfig {
            name: name.to_string(),
            runner,
            workdir: String::new(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
        }
    }

    /// M27: `[[qa.suites]]` rỗng ⇒ không tra được suite nào (fail-closed như D14.1).
    #[test]
    fn empty_catalog_denies_everything() {
        let catalog = SuiteCatalog::default();
        assert!(catalog.is_empty());
        assert!(catalog.get("core-unit").is_none());
    }

    #[test]
    fn catalog_lookup_by_declared_name() {
        let catalog = SuiteCatalog::from_config(&[
            cfg("core-unit", QaRunner::CargoTest, &["--workspace"]),
            cfg("web", QaRunner::Vitest, &[]),
        ]);
        assert!(!catalog.is_empty());
        assert_eq!(catalog.names(), ["core-unit", "web"]);
        assert_eq!(
            catalog.get("core-unit").unwrap().runner,
            QaRunner::CargoTest
        );
        assert!(catalog.get("khong-ton-tai").is_none());
    }

    /// `workdir` rỗng = gốc workspace, phải chuẩn hoá thành `.` — nếu không,
    /// `run_argv_readonly` sẽ dựng `--workdir ""` và docker báo lỗi khó hiểu.
    #[test]
    fn empty_workdir_is_normalised_to_dot() {
        let catalog = SuiteCatalog::from_config(&[cfg("core-unit", QaRunner::CargoTest, &[])]);
        assert_eq!(catalog.get("core-unit").unwrap().workdir, ".");
    }

    /// Model **không** chọn được chương trình: argv do code dựng theo runner.
    #[test]
    fn argv_is_built_by_code_per_runner() {
        let fixed = vec!["--workspace".to_string()];
        assert_eq!(
            build_argv(QaRunner::CargoTest, &fixed, Some("test_ok")),
            ["cargo", "test", "--workspace", "--", "test_ok"]
        );
        assert_eq!(
            build_argv(QaRunner::Vitest, &fixed, None),
            ["pnpm", "exec", "vitest", "run", "--workspace"]
        );
        assert_eq!(
            build_argv(QaRunner::Pytest, &[], Some("test_ok")),
            ["python", "-m", "pytest", "-k", "test_ok"]
        );
    }

    /// `cargo test` đặt filter sau `--` ⇒ không bao giờ bị parser hiểu nhầm thành tuỳ chọn.
    #[test]
    fn cargo_filter_is_placed_after_double_dash() {
        let argv = build_argv(QaRunner::CargoTest, &[], Some("--locked"));
        let pos = argv.iter().position(|a| a == "--").expect("có --");
        assert_eq!(argv[pos + 1], "--locked");
    }

    #[test]
    fn filter_accepts_normal_test_names() {
        assert_eq!(
            sanitize_filter("test_ok").unwrap(),
            Some("test_ok".to_string())
        );
        assert_eq!(
            sanitize_filter("  core::tools::test_abc  ").unwrap(),
            Some("core::tools::test_abc".to_string())
        );
        assert_eq!(sanitize_filter("").unwrap(), None);
        assert_eq!(sanitize_filter("   ").unwrap(), None);
    }

    /// Payload shell trong `filter` bị từ chối ở ranh giới — không chỉ "vô hại vì không có
    /// shell" (đó là lớp phụ; ranh giới này bảo vệ dữ liệu đi vào báo cáo).
    #[test]
    fn filter_rejects_shell_metacharacters() {
        for payload in [
            "; rm -rf /",
            "a && b",
            "$(whoami)",
            "`id`",
            "a | b",
            "a > b",
            "a'b",
            "a\"b",
            "a\\b",
            "a\nb",
            "a b",
        ] {
            assert!(
                sanitize_filter(payload).is_err(),
                "phải từ chối `{payload}`"
            );
        }
    }

    #[test]
    fn filter_rejects_overlong_input() {
        let long = "a".repeat(MAX_FILTER_CHARS + 1);
        assert!(sanitize_filter(&long).is_err());
        let ok = "a".repeat(MAX_FILTER_CHARS);
        assert!(sanitize_filter(&ok).is_ok());
    }
}
