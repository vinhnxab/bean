//! `bean update` — kiểm tra và cài bản mới từ GitHub Releases (M30, D23).
//!
//! Flow được ghép từ các bước nguyên tử của `bean-update` để in tiến trình rõ
//! ràng cho người dùng (mỗi bước một dòng):
//!
//! 1. `check` — hỏi GitHub bản stable mới nhất (chỉ hỏi, không tải).
//! 2. `download_and_verify` — tải artifact + `SHA256SUMS`, so sha256.
//! 3. `extract_binary` — giải nén, chặn tar-slip.
//! 4. `install_binary` — copy vào thư mục binary rồi rename đè (atomic; Windows
//!    đi qua `bean.exe.old`).
//! 5. restart — tự gọi `scripts/beanctl restart` (người dùng chủ chốt
//!    2026-11-08); không tìm thấy beanctl (hoặc Windows) thì in hướng dẫn tay.
//!
//! Lệnh này KHÔNG đọc `bean.toml` — updater không phụ thuộc cấu hình; nguồn
//! phát hành là hằng `bean_update::GITHUB_REPO` (D23.1).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};

/// Tham số của `bean update` (xem `cli.rs` cho cờ `--check`/`--no-restart`).
#[derive(Debug, clap::Args)]
pub struct UpdateArgs {
    /// Chỉ kiểm tra có bản mới, không tải hay cài.
    #[arg(long)]
    pub check: bool,

    /// Sau khi cài xong KHÔNG tự restart Bean.
    #[arg(long)]
    pub no_restart: bool,
}

/// Thời gian chờ `beanctl restart`: script chờ SIGTERM tối đa 15s + sleep 3s,
/// giữ biên 180s cho máy chậm.
const RESTART_TIMEOUT: Duration = Duration::from_secs(180);

pub async fn run(args: &UpdateArgs) -> anyhow::Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!(
        "Bean {current} — nguồn cập nhật: GitHub `{}` (chỉ bản stable)",
        bean_update::GITHUB_REPO
    );
    let client = bean_update::http_client();
    let check = bean_update::check(&client, current).await.map_err(|error| {
        anyhow::anyhow!("không kiểm tra được bản mới: {error}\n→ kiểm tra mạng rồi thử lại; xem bản phát hành tại https://github.com/{}/releases", bean_update::GITHUB_REPO)
    })?;

    if !check.update_available {
        println!("✓ Bạn đang dùng bản mới nhất ({}).", check.current);
        return Ok(());
    }

    println!(
        "⬆ Có bản mới: {} → {}\n  ghi chú: {}\n  trang release: {}",
        check.current,
        check.latest.tag,
        first_line_or_dash(&check.latest.notes),
        check.latest.url
    );

    if args.check {
        println!("(chỉ kiểm tra — chạy `bean update` để tải và cài)");
        return Ok(());
    }

    let Some(artifact) = check.artifact.as_deref() else {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        bail!(
            "nền tảng {os}/{arch} chưa có artifact trong release — hãy tải tay từ {}",
            check.latest.url
        );
    };

    println!("↓ Đang tải {artifact} + SHA256SUMS…");
    let (dir, archive) = bean_update::download_and_verify(&client, &check.latest, artifact)
        .await
        .context("tải hoặc xác minh artifact thất bại (không ghi gì lên binary cũ)")?;
    println!("✓ Đã tải và khớp SHA256.");

    let dest = dir.path().join("extracted");
    tokio::fs::create_dir_all(&dest).await?;
    let staged = {
        let archive = archive.clone();
        let dest = dest.clone();
        tokio::task::spawn_blocking(move || bean_update::extract_binary(&archive, &dest))
            .await
            .context("lỗi thread pool khi giải nén")??
    };
    println!("✓ Đã giải nén binary mới.");

    let target =
        std::env::current_exe().context("không xác định được đường dẫn binary đang chạy")?;
    {
        let staged = staged.clone();
        let target = target.clone();
        tokio::task::spawn_blocking(move || bean_update::install_binary(&staged, &target))
            .await
            .context("lỗi thread pool khi ghi đè binary")??;
    }
    println!("✓ Đã cài {} vào {}.", check.latest.tag, target.display());

    if args.no_restart {
        println!(
            "Bỏ qua restart (--no-restart). Chạy `scripts/beanctl restart` để áp dụng bản mới."
        );
        return Ok(());
    }
    restart_after_install(&target).await
}

/// Dòng đầu **có nội dung** của ghi chú release, hoặc `—` khi rỗng — ghi chú
/// release thường mở đầu bằng dòng trống, giữ output CLI gọn.
fn first_line_or_dash(notes: &str) -> String {
    notes
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("—")
        .to_string()
}

/// Tự restart sau khi cài (D23.5): tìm `scripts/beanctl` đi ngược từ thư mục
/// binary (repo layout `target/release/bean` → `scripts/beanctl`), không thấy
/// thì in hướng dẫn tay. Chỉ chạy trên Unix — `beanctl` là bash script.
async fn restart_after_install(target: &Path) -> anyhow::Result<()> {
    #[cfg(not(unix))]
    {
        let _ = target;
        println!("→ Windows chưa có beanctl: mở terminal mới và chạy lại Bean để dùng bản mới.");
        return Ok(());
    }
    #[cfg(unix)]
    {
        let Some(beanctl) = find_beanctl(target) else {
            println!(
                "→ Không tìm thấy scripts/beanctl: khởi động lại Bean bằng cách bạn vẫn chạy để áp dụng bản mới."
            );
            return Ok(());
        };
        println!("⟳ Đang restart Bean qua {}…", beanctl.display());
        let mut command = tokio::process::Command::new(&beanctl);
        command.arg("restart");
        let status = tokio::time::timeout(RESTART_TIMEOUT, command.status())
            .await
            .context("beanctl restart vượt quá 180s")?
            .with_context(|| format!("không chạy được {}", beanctl.display()))?;
        if status.success() {
            println!("✓ Bean đã chạy lại với bản mới.");
            Ok(())
        } else {
            bail!("beanctl restart thất bại ({status}) — xem log trong /tmp/bean-run/serve.log");
        }
    }
}

/// Đi ngược tối đa 6 cấp từ thư mục binary để tìm `scripts/beanctl` — đủ sâu
/// cho layout `repo/target/release/bean` và `repo/target/debug/bean`.
#[cfg(unix)]
fn find_beanctl(target: &Path) -> Option<PathBuf> {
    let mut dir = target.parent();
    for _ in 0..6 {
        let current = dir?;
        let candidate = current.join("scripts").join("beanctl");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = current.parent();
    }
    None
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn find_beanctl_walks_up_to_repo_layout() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join("scripts")).unwrap();
        std::fs::create_dir_all(repo.path().join("target/release")).unwrap();
        let beanctl = repo.path().join("scripts/beanctl");
        std::fs::write(&beanctl, "#!/usr/bin/env bash\n").unwrap();
        let fake_bin = repo.path().join("target/release/bean");
        std::fs::write(&fake_bin, b"bin").unwrap();
        assert_eq!(find_beanctl(&fake_bin), Some(beanctl));
    }

    #[test]
    fn find_beanctl_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let fake_bin = dir.path().join("bean");
        std::fs::write(&fake_bin, b"bin").unwrap();
        assert_eq!(find_beanctl(&fake_bin), None);
    }

    #[test]
    fn first_line_or_dash_handles_empty_and_multiline() {
        assert_eq!(first_line_or_dash(""), "—");
        assert_eq!(first_line_or_dash("\n\nx"), "x");
        assert_eq!(first_line_or_dash("dòng một\ndòng hai"), "dòng một");
    }
}
