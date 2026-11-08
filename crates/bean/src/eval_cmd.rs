//! `bean eval` — chạy bộ eval đo chất lượng agent (`bean-eval`).
//!
//! Đây là phần "đo được chất lượng" mà test đơn vị không thay thế được: test
//! chứng minh *code* đúng theo kịch bản dựng sẵn, eval chứng minh *agent*
//! hoàn thành tác vụ với số đo được so sánh trước/sau khi tối ưu. Lần đầu chạy
//! nên lấy đường cơ sở (`evals/out/metrics.jsonl`) rồi giữ lại để đối chiếu.
//!
//! Task fake (kịch bản) xác định và miễn phí — chạy mọi lúc, cả CI. Task
//! provider thật là **opt-in**: key chỉ đọc từ biến môi trường (`--provider
//! real` với task khai báo `api_key_env`), không bao giờ từ file trong repo.

use anyhow::{Context, Result};

use crate::cli::{EvalArgs, EvalCommand};

/// Điểm vào `bean eval`; trả mã thoát (0 = mọi task pass).
///
/// # Errors
/// Lỗi khi nạp task/thư mục out/file metrics. Task fail **không** phải lỗi
/// lệnh — nó được in vào báo cáo và phản ánh qua mã thoát.
pub async fn run(args: &EvalArgs) -> Result<i32> {
    match &args.command {
        EvalCommand::Run {
            tasks,
            only,
            provider,
            out,
        } => run_all(tasks, only.as_deref(), provider.as_deref(), out).await,
        EvalCommand::Report { file } => report(file).map(|()| 0),
    }
}

async fn run_all(
    tasks_dir: &std::path::Path,
    only: Option<&str>,
    provider: Option<&str>,
    out_dir: &std::path::Path,
) -> Result<i32> {
    let choice = match provider {
        None => bean_eval::ProviderChoice::Auto,
        Some("fake") => bean_eval::ProviderChoice::Fake,
        Some("real") => bean_eval::ProviderChoice::Real,
        Some(other) => anyhow::bail!("--provider chỉ nhận `fake` hoặc `real`, nhận `{other}`"),
    };
    let options = bean_eval::RunOptions {
        provider: choice,
        out_dir: out_dir.to_path_buf(),
    };
    let results = bean_eval::run_tasks_in_dir(tasks_dir, only, &options)
        .await
        .with_context(|| format!("chạy task trong {}", tasks_dir.display()))?;
    let records: Vec<bean_eval::MetricsRecord> = results
        .iter()
        .map(bean_eval::MetricsRecord::from_result)
        .collect();
    bean_eval::print_report(&records).context("in báo cáo eval")?;
    println!(
        "\nmetrics: {} · trajectory: {}",
        out_dir.join("metrics.jsonl").display(),
        out_dir.join("trajectory").display()
    );
    let failed = results.iter().filter(|result| !result.ok).count();
    Ok(i32::from(failed > 0))
}

fn report(file: &std::path::Path) -> Result<()> {
    let records = bean_eval::MetricsRecord::read_all(file)
        .with_context(|| format!("đọc metrics {}", file.display()))?;
    if records.is_empty() {
        anyhow::bail!("{} không có record nào", file.display());
    }
    bean_eval::print_report(&records).context("in báo cáo eval")?;
    Ok(())
}
