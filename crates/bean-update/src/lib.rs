//! # bean-update — kiểm tra và cài bản cập nhật từ GitHub Releases (M30)
//!
//! Cơ chế update "như phần mềm thật" cho Bean, theo D23 trong `docs/decisions.md`:
//!
//! * **Nguồn phát hành:** GitHub Releases của repo `vinhnxab/bean` (người dùng chủ
//!   chốt phương án A, 2026-11-08). `GET /releases/latest` của GitHub **loại sẵn**
//!   draft và prerelease — khớp quyết định "chỉ nhận bản stable" (D23.7).
//! * **Check tay thôi:** không phone-home, không check định kỳ. Hai đường vào:
//!   CLI `bean update [--check]` và nút "Kiểm tra bản mới" trong UI (endpoint
//!   `GET /api/update/check`, admin-only). UI **không** self-update ở v1 (D23.6).
//! * **Toàn vẹn:** tải kèm `SHA256SUMS`, so sha256 trước khi cài (D23.3). Không
//!   có chữ ký GPG ở v1 — rủi ro đã ghi rõ trong D23.3.
//! * **Một định dạng artifact:** `.tar.gz` cho mọi nền tảng (Windows runner vẫn
//!   có `tar`/bsdtar — D23.2), nên chỉ cần một đường giải nén.
//! * **Thay binary an toàn:** Unix copy vào cùng thư mục rồi `rename` đè (atomic,
//!   tiến trình đang chạy giữ inode cũ tới khi restart); Windows đổi binary đang
//!   chạy sang `bean.exe.old` rồi đặt bản mới vào chỗ cũ (D23.4).
//! * **Tự restart:** sau khi cài xong, CLI tự gọi `scripts/beanctl restart`
//!   (người dùng chủ chốt, 2026-11-08); không tìm thấy beanctl thì in hướng dẫn
//!   tay — phần đó thuộc bin crate, crate này chỉ cung cấp các bước nguyên tử.

use std::cmp::Ordering;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Repo phát hành trên GitHub — nguồn duy nhất của bản update (D23.1).
pub const GITHUB_REPO: &str = "vinhnxab/bean";

/// Trần dung lượng artifact (512 MiB) — phản vệ khi release bị bẩn hoặc bị
/// điều hướng tới URL lạ.
pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

/// Trần dung lượng file `SHA256SUMS` (1 MiB) — file vài trăm byte là cùng.
const MAX_CHECKSUM_BYTES: u64 = 1024 * 1024;

/// Cắt ghi chú release khi trả về UI/CLI.
const NOTES_MAX_CHARS: usize = 1000;

const CHECKSUMS_ASSET: &str = "SHA256SUMS";

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("lỗi mạng khi truy cập GitHub: {0}")]
    Http(#[from] reqwest::Error),
    #[error("GitHub trả {status}: {message}")]
    Api { status: u16, message: String },
    #[error("chưa có bản stable nào được phát hành cho {repo}")]
    NoStableRelease { repo: String },
    #[error("nền tảng {os}/{arch} chưa có artifact cập nhật — hãy tải tay từ trang release")]
    PlatformUnsupported { os: String, arch: String },
    #[error("bản {tag} không có artifact {artifact}")]
    ArtifactMissing { tag: String, artifact: String },
    #[error("file {0} thiếu, quá lớn hoặc không có mục cho artifact")]
    ChecksumFile(String),
    #[error("sai mã SHA256 của artifact: mong đợi {expected}, thực tế {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("artifact quá lớn ({0} byte, giới hạn {MAX_DOWNLOAD_BYTES})")]
    DownloadTooLarge(u64),
    #[error("archive chứa đường dẫn không an toàn: {0}")]
    UnsafeArchive(String),
    #[error("archive không chứa binary Bean (mong đợi bean hoặc bean.exe ở cấp gốc)")]
    BinaryMissing,
    #[error("lỗi file hệ thống: {0}")]
    Io(#[from] std::io::Error),
}

/// Bản stable mới nhất trên GitHub Releases, đã map sang kiểu nội bộ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Tag (ví dụ `v0.2.0`).
    pub tag: String,
    /// Trang release trên GitHub.
    pub url: String,
    /// Ghi chú release (body), cắt ở `NOTES_MAX_CHARS` ký tự.
    pub notes: String,
    /// Danh sách file đính kèm.
    pub assets: Vec<ReleaseAsset>,
}

/// Một file đính kèm của release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    /// URL tải trực tiếp (`browser_download_url` của GitHub).
    pub url: String,
    pub size: u64,
}

/// Kết quả `check()` — chỉ hỏi, không tải.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    /// Phiên bản binary đang chạy.
    pub current: String,
    /// Bản stable mới nhất.
    pub latest: Release,
    /// `latest.tag` > `current`.
    pub update_available: bool,
    /// Artifact cho nền tảng hiện tại; `None` khi nền tảng chưa được build.
    pub artifact: Option<String>,
}

// --- GitHub API -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GhRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// HTTP client dùng chung cho mọi lượt gọi GitHub: User-Agent bắt buộc của API,
/// timeout vừa phải (lượt tải artifact nới riêng ở `download_to_file`).
#[must_use]
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("bean/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_default()
}

/// Tên artifact cho nền tảng đang biên dịch; `None` khi nền tảng chưa có bản
/// build trong workflow release (D23.2).
#[must_use]
pub fn platform_artifact() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("bean-x86_64-unknown-linux-gnu.tar.gz"),
        ("linux", "aarch64") => Some("bean-aarch64-unknown-linux-gnu.tar.gz"),
        ("macos", "x86_64") => Some("bean-x86_64-apple-darwin.tar.gz"),
        ("macos", "aarch64") => Some("bean-aarch64-apple-darwin.tar.gz"),
        ("windows", "x86_64") => Some("bean-x86_64-pc-windows-msvc.tar.gz"),
        _ => None,
    }
}

/// Hỏi GitHub bản stable mới nhất (đường dẫn thật, dùng bởi CLI/web).
pub async fn fetch_latest(client: &reqwest::Client) -> Result<Release, UpdateError> {
    fetch_latest_from(
        client,
        &format!("https://api.github.com/repos/{GITHUB_REPO}"),
    )
    .await
}

/// Bản test-able: `api_base` truyền được để trỏ vào server giả (wiremock).
async fn fetch_latest_from(
    client: &reqwest::Client,
    api_base: &str,
) -> Result<Release, UpdateError> {
    let url = format!("{api_base}/releases/latest");
    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(UpdateError::Api {
            status: status.as_u16(),
            message: format!("{url} trả {status}"),
        });
    }
    let gh: GhRelease = response.json().await?;
    // Endpoint `/releases/latest` của GitHub đã loại draft/prerelease, nhưng vẫn
    // kiểm tra cờ để không phụ thuộc hành vi bên ngoài (D23.7).
    if gh.draft || gh.prerelease || gh.tag_name.is_empty() {
        return Err(UpdateError::NoStableRelease {
            repo: GITHUB_REPO.to_string(),
        });
    }
    Ok(Release {
        // Bỏ tiền tố `v` để mọi bên tiêu dùng (DTO web, CLI, i18n template có
        // sẵn "v" trong chuỗi) không phải tự đoán; `compare_versions` vốn đã
        // bỏ tiền tố nên hành vi so sánh không đổi.
        tag: gh.tag_name.trim_start_matches(['v', 'V']).to_string(),
        url: gh.html_url,
        notes: truncate_chars(&gh.body, NOTES_MAX_CHARS),
        assets: gh
            .assets
            .into_iter()
            .map(|asset| ReleaseAsset {
                name: asset.name,
                url: asset.browser_download_url,
                size: asset.size,
            })
            .collect(),
    })
}

/// So sánh phiên bản kiểu semver-đơn giản: `0.10.0` > `0.9.0`, bỏ tiền tố `v`.
/// Phần không phải số trong một đoạn (ví dụ `-rc1`) so theo chuỗi — chỉ mang
/// tính phụ vì artifact chỉ phát hành bản stable.
#[must_use]
pub fn compare_versions(current: &str, latest_tag: &str) -> Ordering {
    fn segments(version: &str) -> Vec<(u64, String)> {
        let trimmed = version.trim().trim_start_matches(['v', 'V']);
        trimmed
            .split('.')
            .map(|segment| {
                let digits: usize = segment
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .map(char::len_utf8)
                    .sum();
                let number = segment[..digits].parse().unwrap_or(0);
                (number, segment[digits..].to_string())
            })
            .collect()
    }
    let (left, right) = (segments(current), segments(latest_tag));
    for index in 0..left.len().max(right.len()) {
        let (ln, lr) = left.get(index).cloned().unwrap_or((0, String::new()));
        let (rn, rr) = right.get(index).cloned().unwrap_or((0, String::new()));
        if ln != rn {
            return ln.cmp(&rn);
        }
        if lr != rr {
            return lr.cmp(&rr);
        }
    }
    Ordering::Equal
}

/// Kiểm tra có bản mới không — chỉ gọi API, không tải artifact.
pub async fn check(
    client: &reqwest::Client,
    current_version: &str,
) -> Result<UpdateCheck, UpdateError> {
    let artifact = platform_artifact().map(str::to_string);
    let latest = fetch_latest(client).await?;
    let update_available = compare_versions(current_version, &latest.tag) == Ordering::Less;
    Ok(UpdateCheck {
        current: current_version.to_string(),
        latest,
        update_available,
        artifact,
    })
}

// --- Tải và xác minh -------------------------------------------------------------

/// Tải artifact cho `platform` (ví dụ `bean-x86_64-unknown-linux-gnu.tar.gz`) kèm
/// `SHA256SUMS`, xác minh sha256, trả về `(TempDir, đường dẫn artifact)`. TempDir
/// phải được giữ sống tới khi `extract_binary` chạy xong.
pub async fn download_and_verify(
    client: &reqwest::Client,
    release: &Release,
    platform: &str,
) -> Result<(tempfile::TempDir, PathBuf), UpdateError> {
    let artifact = release
        .assets
        .iter()
        .find(|asset| asset.name == platform)
        .ok_or_else(|| UpdateError::ArtifactMissing {
            tag: release.tag.clone(),
            artifact: platform.to_string(),
        })?;
    let sums = release
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUMS_ASSET)
        .ok_or_else(|| UpdateError::ChecksumFile(CHECKSUMS_ASSET.to_string()))?;
    let expected = expected_digest(client, &sums.url, platform).await?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join(platform);
    let actual = download_to_file(client, &artifact.url, &path, MAX_DOWNLOAD_BYTES).await?;
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(UpdateError::ChecksumMismatch { expected, actual });
    }
    Ok((dir, path))
}

/// Tải `SHA256SUMS`, trả về mã hex cho `platform` (dòng có dạng `<hex>  <tên>`).
async fn expected_digest(
    client: &reqwest::Client,
    sums_url: &str,
    platform: &str,
) -> Result<String, UpdateError> {
    let text = download_to_string(client, sums_url, MAX_CHECKSUM_BYTES).await?;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(digest), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name == platform
            && digest.len() == 64
            && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Ok(digest.to_ascii_lowercase());
        }
    }
    Err(UpdateError::ChecksumFile(CHECKSUMS_ASSET.to_string()))
}

/// Tải file nhị phân về `dest` theo chunk, đồng thời tính sha256 — trả về hex.
/// Trần `max_bytes` chặn từ cả `Content-Length` lẫn đếm thực tế khi nhận.
async fn download_to_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    max_bytes: u64,
) -> Result<String, UpdateError> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(600))
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(UpdateError::Api {
            status: status.as_u16(),
            message: format!("{url} trả {status}"),
        });
    }
    if let Some(length) = response.content_length()
        && length > max_bytes
    {
        return Err(UpdateError::DownloadTooLarge(length));
    }
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut response = response;
    while let Some(chunk) = response.chunk().await? {
        total = total.saturating_add(chunk.len() as u64);
        if total > max_bytes {
            return Err(UpdateError::DownloadTooLarge(total));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(hex_lower(&hasher.finalize()))
}

/// Tải file văn bản nhỏ (SHA256SUMS) về chuỗi, có trần dung lượng.
async fn download_to_string(
    client: &reqwest::Client,
    url: &str,
    max_bytes: u64,
) -> Result<String, UpdateError> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(60))
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(UpdateError::Api {
            status: status.as_u16(),
            message: format!("{url} trả {status}"),
        });
    }
    let bytes = response.bytes().await?;
    if bytes.len() as u64 > max_bytes {
        return Err(UpdateError::DownloadTooLarge(bytes.len() as u64));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|error| UpdateError::ChecksumFile(format!("SHA256SUMS không phải UTF-8: {error}")))
}

fn hex_lower(digest: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// --- Giải nén và ghi đè binary ---------------------------------------------------

/// Giải nén `.tar.gz` và ghi binary (`bean` trên Unix, `bean.exe` trên Windows)
/// vào `dest_dir`, trả về đường dẫn binary. Chỉ nhận **regular file**; bỏ qua
/// mọi mục khác (LICENSE, thư mục con…); chặn đường dẫn tuyệt đối và `..`
/// trước khi chạm đĩa (tar-slip — D23.3).
pub fn extract_binary(archive: &Path, dest_dir: &Path) -> Result<PathBuf, UpdateError> {
    let file = std::fs::File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let binary_name: &str = if cfg!(windows) { "bean.exe" } else { "bean" };
    let mut found: Option<PathBuf> = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        validate_entry_path(&path)?;
        if !entry_is_regular(&entry) {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        if name.as_deref() == Some(binary_name) {
            let dest = dest_dir.join(binary_name);
            entry.unpack(&dest)?;
            found = Some(dest);
        }
    }
    found.ok_or(UpdateError::BinaryMissing)
}

/// Chặn đường dẫn nguy hiểm trong archive: tuyệt đối hoặc chứa `..`.
fn validate_entry_path(path: &Path) -> Result<(), UpdateError> {
    let traverses = path
        .components()
        .any(|component| component == std::path::Component::ParentDir);
    if path.is_absolute() || traverses {
        return Err(UpdateError::UnsafeArchive(
            path.to_string_lossy().into_owned(),
        ));
    }
    Ok(())
}

fn entry_is_regular<R: Read>(entry: &tar::Entry<'_, R>) -> bool {
    matches!(
        entry.header().entry_type(),
        tar::EntryType::Regular | tar::EntryType::GNUSparse
    )
}

/// Ghi đè binary đang chạy bằng bản mới (D23.4). Xem doc của
/// `install_replace` về khác biệt Unix/Windows.
pub fn install_binary(new_bin: &Path, target: &Path) -> Result<(), UpdateError> {
    let dir = target
        .parent()
        .ok_or_else(|| UpdateError::Io(std::io::Error::other("binary không có thư mục cha")))?;
    let staged = dir.join(if cfg!(windows) {
        "bean.new.exe"
    } else {
        "bean.new"
    });
    // Copy (không rename) từ thư mục tạm: /tmp có thể khác filesystem với thư
    // mục binary, `rename` giữa hai filesystem sẽ trả EXDEV.
    std::fs::copy(new_bin, &staged)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    install_replace(&staged, target)
}

#[cfg_attr(windows, allow(clippy::needless_return))]
fn install_replace(staged: &Path, target: &Path) -> Result<(), UpdateError> {
    #[cfg(windows)]
    {
        // Windows không cho xoá/ghi đè exe đang chạy nhưng cho **đổi tên** nó:
        // `bean.exe` → `bean.exe.old`, đặt bản mới vào chỗ cũ. Lỗi giữa chừng
        // thì trả `.old` về chỗ cũ — máy không được rơi vào trạng thái không có
        // binary. `.old` thừa sẽ được dọn ở lần update sau.
        let old = target.with_extension("exe.old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(target, &old)?;
        if let Err(error) = std::fs::rename(staged, target) {
            let _ = std::fs::rename(&old, target);
            return Err(error.into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        // Rename trong cùng thư mục = atomic; tiến trình đang chạy giữ inode cũ
        // tới khi thoát — an toàn ngay cả khi `bean update` chạy trong lúc
        // `bean serve` đang hoạt động.
        std::fs::rename(staged, target)?;
        Ok(())
    }
}

/// Cắt chuỗi ở ranh giới ký tự (UTF-8 safe) — dùng cho ghi chú release.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max_chars).collect::<String>())
}

// --- Test ------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn compare_versions_semantic() {
        use Ordering::{Equal, Greater, Less};
        assert_eq!(compare_versions("0.1.0", "v0.2.0"), Less);
        assert_eq!(compare_versions("0.10.0", "v0.9.0"), Greater);
        assert_eq!(compare_versions("v0.1.0", "0.1.0"), Equal);
        assert_eq!(compare_versions("0.2.0", "v0.2.1"), Less);
        assert_eq!(compare_versions("1.0.0", "v0.99.0"), Greater);
    }

    #[test]
    fn truncate_chars_is_utf8_safe_and_marks_ellipsis() {
        let text = "tiếng Việt có dấu 🎉 lặp".repeat(100);
        let cut = truncate_chars(&text, 10);
        assert!(cut.chars().count() <= 11);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate_chars("ngắn", 10), "ngắn");
    }

    #[test]
    fn validate_entry_path_blocks_traversal_and_absolute() {
        assert!(validate_entry_path(Path::new("bean")).is_ok());
        assert!(validate_entry_path(Path::new("sub/dir/bean")).is_ok());
        assert!(validate_entry_path(Path::new("../evil")).is_err());
        assert!(validate_entry_path(Path::new("a/../bean")).is_err());
        assert!(validate_entry_path(Path::new("/abs/bean")).is_err());
    }

    #[test]
    fn platform_artifact_matches_current_host() {
        let artifact = platform_artifact();
        let supported = matches!(
            (std::env::consts::OS, std::env::consts::ARCH),
            ("linux", "x86_64" | "aarch64")
                | ("macos", "x86_64" | "aarch64")
                | ("windows", "x86_64")
        );
        if supported {
            let name = artifact.expect("nền tảng này phải có artifact");
            assert!(name.starts_with("bean-"));
            assert!(name.ends_with(".tar.gz"));
            assert!(name.contains(std::env::consts::ARCH));
        }
    }

    #[tokio::test]
    async fn fetch_latest_maps_release_and_filters_prerelease() {
        let body_ok = serde_json::json!({
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/vinhnxab/bean/releases/tag/v0.2.0",
            "draft": false,
            "prerelease": false,
            "body": "Ghi chú phát hành 🎉",
            "assets": [
                {"name": "bean-x86_64-unknown-linux-gnu.tar.gz",
                 "browser_download_url": "https://example.com/bean.tar.gz", "size": 10},
                {"name": "SHA256SUMS", "browser_download_url": "https://example.com/SUMS", "size": 2}
            ]
        });
        let body_pre = serde_json::json!({
            "tag_name": "v0.3.0-rc1", "draft": false, "prerelease": true, "assets": []
        });
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/releases/latest"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body_ok))
            .mount(&server)
            .await;
        let release = fetch_latest_from(&http_client(), &server.uri())
            .await
            .unwrap();
        // Tag từ GitHub là `v0.2.0` — lõi đã strip tiền tố `v` cho đồng nhất
        // với `CARGO_PKG_VERSION` (không strip thì UI render "vv0.2.0").
        assert_eq!(release.tag, "0.2.0");
        assert_eq!(release.assets.len(), 2);
        assert!(release.notes.contains('🎉'));

        let server_pre = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/releases/latest"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body_pre))
            .mount(&server_pre)
            .await;
        assert!(matches!(
            fetch_latest_from(&http_client(), &server_pre.uri()).await,
            Err(UpdateError::NoStableRelease { .. })
        ));
    }

    #[tokio::test]
    async fn fetch_latest_maps_http_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/releases/latest"))
            .respond_with(wiremock::ResponseTemplate::new(403))
            .mount(&server)
            .await;
        match fetch_latest_from(&http_client(), &server.uri()).await {
            Err(UpdateError::Api { status, .. }) => assert_eq!(status, 403),
            other => panic!("kỳ vọng Api lỗi, thấy: {other:?}"),
        }
    }

    /// Dựng `.tar.gz` hợp lệ bằng tar crate: binary ở gốc + file thường khác.
    fn build_archive(dir: &Path) -> PathBuf {
        let archive_path = dir.join("artifact.tar.gz");
        let file = std::fs::File::create(&archive_path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);

        let mut header = tar::Header::new_gnu();
        header.set_size(6);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "bean", b"NEWBIN" as &[u8])
            .unwrap();

        let mut header_doc = tar::Header::new_gnu();
        header_doc.set_size(4);
        header_doc.set_mode(0o644);
        header_doc.set_cksum();
        builder
            .append_data(&mut header_doc, "docs/readme.txt", b"doc!" as &[u8])
            .unwrap();

        builder.into_inner().unwrap().finish().unwrap();
        archive_path
    }

    #[test]
    fn extract_binary_extracts_only_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let archive = build_archive(dir.path());
        let dest = tempfile::tempdir().unwrap();
        let binary = extract_binary(&archive, dest.path()).unwrap();
        assert_eq!(binary, dest.path().join("bean"));
        assert_eq!(std::fs::read(&binary).unwrap(), b"NEWBIN");
        assert!(!dest.path().join("docs").exists());
    }

    /// Lập một entry ustar thô (512-byte header + data) — dùng để kiểm chứng
    /// extract_binary chặn đường dẫn `..` lúc ĐỌC, vì tar crate từ chối ghi
    /// đường dẫn đó lúc đóng gói (lớp phòng thủ thứ hai phải tự đứng vững).
    fn raw_ustar_entry(name: &str, content: &[u8]) -> Vec<u8> {
        let mut block = [0u8; 512];
        block[..name.len()].copy_from_slice(name.as_bytes());
        block[100..107].copy_from_slice(b"0000644");
        block[108..115].copy_from_slice(b"0000000");
        block[116..123].copy_from_slice(b"0000000");
        block[124..135].copy_from_slice(format!("{:011o}", content.len()).as_bytes());
        block[136..147].copy_from_slice(b"00000000000");
        for byte in &mut block[148..156] {
            *byte = b' ';
        }
        block[156] = b'0';
        block[257..262].copy_from_slice(b"ustar");
        block[263..265].copy_from_slice(b"00");
        let sum: u32 = block.iter().map(|byte| u32::from(*byte)).sum();
        block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        let mut out = block.to_vec();
        out.extend_from_slice(content);
        let pad = (512 - content.len() % 512) % 512;
        out.extend(std::iter::repeat_n(0u8, pad));
        out
    }

    #[test]
    fn extract_binary_rejects_traversal_entries() {
        // Archive thô: entry hợp lệ "bean" + entry `../evil` — header ustar
        // dựng tay nên tar crate không được phép chặn trước.
        let mut raw = raw_ustar_entry("bean", b"NEWBIN");
        raw.extend(raw_ustar_entry("../evil", b"bad!"));
        raw.extend(vec![0u8; 1024]);
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("evil.tar.gz");
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
            std::io::Write::write_all(&mut encoder, &raw).unwrap();
            std::io::Write::flush(&mut encoder).unwrap();
        }
        let dest = tempfile::tempdir().unwrap();
        match extract_binary(&archive_path, dest.path()) {
            Err(UpdateError::UnsafeArchive(path)) => assert!(path.contains("..")),
            other => panic!("kỳ vọng UnsafeArchive, thấy: {other:?}"),
        }
    }

    #[test]
    fn extract_binary_reports_missing_binary() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("empty.tar.gz");
        let file = std::fs::File::create(&archive_path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_cksum();
        builder
            .append_data(&mut header, "other.txt", b"hi!" as &[u8])
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        let dest = tempfile::tempdir().unwrap();
        assert!(matches!(
            extract_binary(&archive_path, dest.path()),
            Err(UpdateError::BinaryMissing)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn install_binary_replaces_target_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("bean");
        std::fs::write(&target, b"OLD").unwrap();
        let new_bin = dir.path().join("staged-src");
        std::fs::write(&new_bin, b"NEW").unwrap();
        install_binary(&new_bin, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"NEW");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        assert!(!dir.path().join("bean.new").exists());
    }

    /// Mock server trả SHA256SUMS + artifact; helper dựng release trỏ tới đó.
    async fn mock_release_with_artifact(
        body: &'static [u8],
    ) -> (wiremock::MockServer, Release, String) {
        use sha2::Digest as _;
        let mut hasher = Sha256::new();
        hasher.update(body);
        let digest = hex_lower(&hasher.finalize());
        let sums = format!("{digest}  bean-x86_64-unknown-linux-gnu.tar.gz\n");
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/bean.tar.gz"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_bytes(body.to_vec())
                    .append_header("Content-Type", "application/gzip"),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/SHA256SUMS"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(sums.clone()))
            .mount(&server)
            .await;
        let release = Release {
            tag: "v9.9.9".into(),
            url: "https://example.com".into(),
            notes: String::new(),
            assets: vec![
                ReleaseAsset {
                    name: "bean-x86_64-unknown-linux-gnu.tar.gz".into(),
                    url: format!("{}/bean.tar.gz", server.uri()),
                    size: body.len() as u64,
                },
                ReleaseAsset {
                    name: "SHA256SUMS".into(),
                    url: format!("{}/SHA256SUMS", server.uri()),
                    size: sums.len() as u64,
                },
            ],
        };
        (server, release, digest)
    }

    #[tokio::test]
    async fn download_and_verify_accepts_matching_digest() {
        let (_server, release, _digest) = mock_release_with_artifact(b"artifact-bytes").await;
        let (_dir, path) = download_and_verify(
            &http_client(),
            &release,
            "bean-x86_64-unknown-linux-gnu.tar.gz",
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"artifact-bytes");
    }

    #[tokio::test]
    async fn download_and_verify_rejects_tampered_artifact() {
        // SHA256SUMS khớp sha256("good") nhưng artifact trả nội dung khác.
        let mut hasher = Sha256::new();
        hasher.update(b"good");
        let sums = format!(
            "{}  bean-x86_64-unknown-linux-gnu.tar.gz\n",
            hex_lower(&hasher.finalize())
        );
        let sums_len = sums.len();
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/bean.tar.gz"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(b"tampered".to_vec()))
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/SHA256SUMS"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(sums))
            .mount(&server)
            .await;
        let release = Release {
            tag: "v9.9.9".into(),
            url: "https://example.com".into(),
            notes: String::new(),
            assets: vec![
                ReleaseAsset {
                    name: "bean-x86_64-unknown-linux-gnu.tar.gz".into(),
                    url: format!("{}/bean.tar.gz", server.uri()),
                    size: 8,
                },
                ReleaseAsset {
                    name: "SHA256SUMS".into(),
                    url: format!("{}/SHA256SUMS", server.uri()),
                    size: sums_len as u64,
                },
            ],
        };
        assert!(matches!(
            download_and_verify(
                &http_client(),
                &release,
                "bean-x86_64-unknown-linux-gnu.tar.gz"
            )
            .await,
            Err(UpdateError::ChecksumMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn download_and_verify_rejects_missing_artifact_and_bad_sums() {
        let (_server, release, _digest) = mock_release_with_artifact(b"x").await;
        assert!(matches!(
            download_and_verify(&http_client(), &release, "bean-other-platform.tar.gz").await,
            Err(UpdateError::ArtifactMissing { .. })
        ));
        // SHA256SUMS không có mục cho platform → ChecksumFile.
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/SHA256SUMS"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string("deadbeef  other-file\n"),
            )
            .mount(&server)
            .await;
        let release_no_sums = Release {
            tag: "v9.9.9".into(),
            url: "https://example.com".into(),
            notes: String::new(),
            assets: vec![
                ReleaseAsset {
                    name: "bean-x86_64-unknown-linux-gnu.tar.gz".into(),
                    url: format!("{}/bean.tar.gz", server.uri()),
                    size: 1,
                },
                ReleaseAsset {
                    name: "SHA256SUMS".into(),
                    url: format!("{}/SHA256SUMS", server.uri()),
                    size: 23,
                },
            ],
        };
        assert!(matches!(
            download_and_verify(
                &http_client(),
                &release_no_sums,
                "bean-x86_64-unknown-linux-gnu.tar.gz"
            )
            .await,
            Err(UpdateError::ChecksumFile(_))
        ));
    }

    /// Test thật (#[ignore], chạy tay: `cargo test -p bean-update -- --ignored`):
    /// gọi GitHub API thật — cần mạng, không chạy trong `make check`.
    #[tokio::test]
    #[ignore = "gọi GitHub thật — cần mạng"]
    async fn fetch_latest_against_real_github() {
        let release = fetch_latest(&http_client()).await.unwrap();
        // Tag thật luôn có dạng số sau khi strip `v` (bản stable của repo này).
        assert!(!release.tag.starts_with('v'), "tag: {}", release.tag);
        assert!(
            release
                .tag
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        );
    }
}
