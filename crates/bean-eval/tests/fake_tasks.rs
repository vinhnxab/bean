//! Test tích hợp bean-eval: chạy bộ task eval trong `evals/tasks/` bằng
//! FakeProvider — cùng một bộ mà CI chạy. Mọi task phải pass: đây là đường
//! cơ sở cho mọi cải tiến sau này (prompt, tool description, compaction).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Duration;

use bean_eval::{MetricsRecord, ProviderChoice, RunOptions, load_tasks_dir, run_tasks};

/// Thư mục task eval ở gốc repo (`crates/bean-eval/../../evals/tasks`).
fn tasks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../evals/tasks")
        .canonicalize()
        .expect("evals/tasks phải tồn tại trong repo")
}

/// Thư mục task provider thật (opt-in) — chỉ kiểm tra **parse**, không gọi mạng.
#[test]
fn real_tasks_parse_and_are_opt_in() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../evals/tasks-real")
        .canonicalize()
        .expect("evals/tasks-real phải tồn tại trong repo");
    let tasks = load_tasks_dir(&dir).unwrap();
    assert!(tasks.len() >= 2, "cần tối thiểu task K12 + K17");
    for task in &tasks {
        assert!(
            !task.llm.is_fake(),
            "task {} phải là provider thật",
            task.id
        );
        let env_name = task.llm.api_key_env.as_deref().unwrap_or_default();
        assert!(
            !env_name.is_empty(),
            "task {} phải khai báo api_key_env",
            task.id
        );
        assert!(
            !env_name.contains('='),
            "api_key_env là TÊN biến, không phải giá trị key"
        );
        assert!(
            task.timeout_seconds >= 180,
            "provider thật cần trần thời gian lớn"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_fake_tasks_pass_and_metrics_are_written() {
    let tasks = load_tasks_dir(&tasks_dir()).unwrap();
    assert!(
        tasks.len() >= 3,
        "cần ít nhất 3 task eval (multistep, compaction, reflection)"
    );
    let out = tempfile::tempdir().unwrap();
    let options = RunOptions {
        provider: ProviderChoice::Fake,
        out_dir: out.path().to_path_buf(),
    };
    let results = tokio::time::timeout(Duration::from_secs(120), run_tasks(&tasks, None, &options))
        .await
        .expect("eval không được vượt 120s")
        .unwrap();
    assert_eq!(results.len(), tasks.len(), "mọi task phải được chạy");

    for result in &results {
        assert!(
            result.error.is_none(),
            "task {} lỗi run: {:?}",
            result.task_id,
            result.error
        );
        assert!(
            result.ok,
            "task {} phải pass, vi phạm: {:?}",
            result.task_id, result.violations
        );
    }

    // Task compaction phải ghi summary thật (kích hoạt được compaction).
    let compaction = results
        .iter()
        .find(|result| result.task_id.contains("compaction"))
        .expect("phải có task compaction");
    assert!(
        compaction
            .summary
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty()),
        "compaction phải tạo summary khác rỗng"
    );
    assert!(
        compaction.input_tokens > 0,
        "lượt tóm tắt phải được ghi nhận usage"
    );

    // Task reflection phải tạo draft.
    let reflection = results
        .iter()
        .find(|result| result.task_id.contains("reflection"))
        .expect("phải có task reflection");
    assert!(
        reflection.draft_count > 0,
        "reflection phải sinh skill nháp"
    );

    // Metrics JSONL ghi đủ một dòng mỗi task.
    let records = MetricsRecord::read_all(&out.path().join("metrics.jsonl")).unwrap();
    assert_eq!(records.len(), tasks.len(), "mỗi task một dòng metrics");
    let summary = bean_eval::summarize(&records);
    assert_eq!(summary.passed, summary.total, "tất cả record đều PASS");
}
