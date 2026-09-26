//! Tool `security_scan` — quét cổng có kiểm soát phạm vi (Plan.md M23).

use std::sync::Arc;

use beanagent_security::{Sandbox, SandboxError};
use beanagent_tools::{Tool, ToolCtx, ToolError};
use beanagent_types::config::INFRA_SCAN_TAG;
use beanagent_types::{Alert, AlertSeverity, Risk, ToolSpec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::scope::ScanScope;

/// Số mục cổng tối đa mỗi lần quét (chống quét quá rộng gây nghẽn).
const MAX_PORTS: usize = 64;

/// Tham số của `security_scan`.
///
/// **Cố ý KHÔNG có tham số `command`.** Model chỉ mô tả *ý định* ("quét IP này, các cổng
/// này"); mọi argv thực sự do [`build_scanner_argv`] dựng trong code. Đây là ranh giới giữa
/// "agent được yêu cầu quét" và "agent được chạy lệnh tuỳ ý" (D14.5).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanParams {
    /// Địa chỉ IP cần quét, ví dụ `192.168.10.5`. **Chỉ IP viết dạng chữ** — không nhận
    /// hostname, và phải nằm trong `[[infra_scope]]` thì mới chạy được.
    pub target: String,
    /// Danh sách cổng, ví dụ `22,80,443` hoặc `1-1024`. Bỏ trống thì dùng bộ cổng mặc định
    /// của scanner. Chỉ chấp nhận số, dấu phẩy và dấu gạch.
    #[serde(default)]
    pub ports: Option<String>,
}

/// Báo cáo chuẩn hoá mà Manager đọc (schema cố định, `Plan.md` mục 3).
#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    /// `ok` (đã quét) hoặc `refused` (ngoài scope / sai định dạng).
    pub status: &'static str,
    /// Tóm tắt một dòng.
    pub summary: String,
    /// Các rủi ro phát hiện được.
    pub risks: Vec<String>,
    /// Mức nghiêm trọng.
    pub severity: AlertSeverity,
}

/// Các cổng mở ra luôn được coi là rủi ro mức cao.
const SENSITIVE_PORTS: &[u16] = &[
    21, 22, 23, 135, 139, 445, 1433, 1521, 2375, 2376, 3306, 3389, 5432, 5900, 5984, 6379, 9200,
    11211, 27017, 27018, 50070,
];

/// Kiểm tra chuỗi `ports` và trả về dạng chuẩn hoá, hoặc lý do từ chối.
fn validate_ports(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("ports rỗng".into());
    }
    if value.len() > 256 {
        return Err("ports quá dài".into());
    }
    let mut count = 0usize;
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err("có phần cổng rỗng".into());
        }
        count += 1;
        match part.split_once('-') {
            Some((from, to)) => {
                let from: u16 = from
                    .parse()
                    .map_err(|_| format!("cổng `{from}` không phải số"))?;
                let to: u16 = to
                    .parse()
                    .map_err(|_| format!("cổng `{to}` không phải số"))?;
                if from == 0 || to == 0 || from > to {
                    return Err(format!("dải cổng `{part}` không hợp lệ"));
                }
            }
            None => {
                let port: u16 = part
                    .parse()
                    .map_err(|_| format!("cổng `{part}` không phải số"))?;
                if port == 0 {
                    return Err("cổng 0 không hợp lệ".into());
                }
            }
        }
    }
    if count > MAX_PORTS {
        return Err(format!("tối đa {MAX_PORTS} mục cổng mỗi lần quét"));
    }
    Ok(value.to_string())
}

/// Dựng argv của scanner từ giá trị **đã kiểm tra**.
///
/// `--` trước target để scanner không bao giờ hiểu nhầm target là tuỳ chọn.
fn build_scanner_argv(target: &str, ports: Option<&str>) -> Vec<String> {
    let mut argv = vec![
        // Không ping: target trong scope có thể không phản hồi ICMP (firewall).
        "-Pn".to_string(),
        // Quét TCP thuần — không cần quyền raw socket nên chạy được ở non-root.
        "-sT".to_string(),
        "--max-retries".to_string(),
        "1".to_string(),
        "--host-timeout".to_string(),
        "10s".to_string(),
    ];
    if let Some(ports) = ports {
        argv.push("-p".to_string());
        argv.push(ports.to_string());
    }
    argv.push("--".to_string());
    argv.push(target.to_string());
    argv
}

/// Rút cổng đang mở từ output kiểu nmap, ví dụ dòng `22/tcp open ssh`.
fn parse_open_ports(output: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in output.lines() {
        let Some((left, _)) = line.split_once('/') else {
            continue;
        };
        let left = left.trim();
        // Không kiểm `contains("open")` trên cả dòng: cổng `open` còn xuất hiện trong
        // dòng `filtered`/`closed`, chỉ phần trước `/` mới là số cổng thật.
        if left.is_empty() || !left.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if !line.contains("open") || line.contains("closed") || line.contains("filtered") {
            continue;
        }
        if let Ok(port) = left.parse::<u16>() {
            ports.push(port);
        }
    }
    ports
}

/// Chuyển output scanner thành báo cáo chuẩn hoá + mức nghiêm trọng.
fn build_report(target: &str, label: &str, output: &str) -> ScanReport {
    let open = parse_open_ports(output);
    let sensitive: Vec<u16> = open
        .iter()
        .copied()
        .filter(|port| SENSITIVE_PORTS.contains(port))
        .collect();
    let (severity, risks) = if !sensitive.is_empty() {
        (
            AlertSeverity::High,
            sensitive
                .iter()
                .map(|port| format!("cổng nhạy cảm {port} đang mở"))
                .collect(),
        )
    } else if !open.is_empty() {
        (
            AlertSeverity::Medium,
            vec![format!(
                "{} cổng đang mở: {}",
                open.len(),
                open.iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )],
        )
    } else {
        (AlertSeverity::Low, Vec::new())
    };
    let target_desc = if label.is_empty() {
        target.to_string()
    } else {
        format!("{target} ({label})")
    };
    ScanReport {
        status: "ok",
        summary: match severity {
            AlertSeverity::High => format!("quét {target_desc}: có cổng nhạy cảm đang mở"),
            AlertSeverity::Medium => format!("quét {target_desc}: có cổng đang mở"),
            AlertSeverity::Low => format!("quét {target_desc}: không thấy cổng mở"),
        },
        risks,
        severity,
    }
}

/// Lệnh scanner: chương trình + tiền tố argv cố định (không model kiểm soát).
#[derive(Debug, Clone)]
pub struct ScannerCmd {
    /// Chương trình chạy trong container, ví dụ `nmap`.
    pub program: String,
    /// Cờ cố định thêm vào trước.
    pub fixed_args: Vec<String>,
}

impl Default for ScannerCmd {
    fn default() -> Self {
        Self {
            program: "nmap".to_string(),
            fixed_args: Vec::new(),
        }
    }
}

/// Dựng tool `security_scan`.
///
/// `scanner` là tiền tố argv do **cấu hình quyết định**; phần còn lại luôn do
/// [`build_scanner_argv`] sinh, không bao giờ lấy từ tham số của model.
#[must_use]
pub fn security_scan(
    scope: ScanScope,
    sandbox: Arc<Sandbox>,
    scanner: ScannerCmd,
) -> Arc<dyn Tool> {
    let empty_scope = scope.is_empty();
    let mut spec = beanagent_tools::typed_spec::<SecurityScanParams>("security_scan");
    spec.description = if empty_scope {
        "Quét cổng một IP trong phạm vi hạ tầng đã khai báo. HIỆN CHƯA KHAI BÁO PHẠM VI \
         ([[infra_scope]] rỗng) — mọi lần quét sẽ bị từ chối."
    } else {
        "Quét cổng một IP nằm trong [[infra_scope]] đã khai báo (read-only, luôn cần xác nhận)."
    }
    .to_string();
    Arc::new(ScanTool {
        scope,
        sandbox,
        scanner,
        spec,
    })
}

struct ScanTool {
    scope: ScanScope,
    sandbox: Arc<Sandbox>,
    scanner: ScannerCmd,
    spec: ToolSpec,
}

#[async_trait::async_trait]
impl Tool for ScanTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    /// Tóm tắt cho UI/audit: chỉ hiện target, không hiện cả cấu hình scanner.
    fn describe(&self, args: &serde_json::Value) -> String {
        args.get("target")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| self.spec.name.clone(), |target| format!("quét {target}"))
    }

    /// `Dangerous` ⇒ `Policy::decide` trả `allow_in_session: false` ⇒ **luôn hỏi, không có
    /// tuỳ chọn "cho phép trong phiên"** (mục 7.2, yêu cầu M23). Không cần logic riêng.
    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Dangerous
    }

    /// Chỉ role giữ tag `infra-scan` mới thấy/cọp tool này (RBAC M21.4).
    fn required_tags(&self) -> Vec<&str> {
        vec![INFRA_SCAN_TAG]
    }

    /// Output scanner là dữ liệu ngoài lõi: banner mà scanner đọc được từ target do kẻ tấn
    /// công kiểm soát được (mục 15.4 / mục 22.5) — phải bọc untrusted.
    fn marks_untrusted(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params: SecurityScanParams =
            beanagent_tools::typed::deserialize_params(args, &self.spec.parameters)
                .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;

        // (1) KIỂM SCOPE TRƯỚC TIÊN — tầng code, không dựa vào model (M23).
        if !self.scope.allows(&params.target) {
            let reason = if self.scope.is_empty() {
                "chưa khai báo mục `[[infra_scope]]` nào".to_string()
            } else {
                format!("`{}` không nằm trong [[infra_scope]]", params.target)
            };
            let report = ScanReport {
                status: "refused",
                summary: format!("không quét {} — ngoài phạm vi đã khai báo", params.target),
                risks: Vec::new(),
                severity: AlertSeverity::Low,
            };
            let message = format!(
                "TỪ CHỐI: không quét được — {reason}. Chỉ được quét target đã khai báo trong \
                 [[infra_scope]]. Nếu cần quét target này, hãy thêm nó vào BeanAgent.toml rồi \
                 chạy lại; không tự quét vòng qua để kiểm tra cho chắc.\n{}",
                render_json(&report)
            );
            // Bật cờ untrusted: lượt này đã chạm vào nguồn ngoài lõi (mục 15.4).
            ctx.untrusted_seen
                .store(true, std::sync::atomic::Ordering::SeqCst);
            return Ok(wrap(&message));
        }

        // (2) Kiểm định dạng ports rồi **tự dựng** argv (D14.5).
        let ports = match params.ports.as_deref() {
            Some(raw) => match validate_ports(raw) {
                Ok(value) => Some(value),
                Err(reason) => {
                    return Err(ToolError::InvalidArgs(format!(
                        "ports không hợp lệ: {reason}"
                    )));
                }
            },
            None => None,
        };
        let mut argv = Vec::with_capacity(self.scanner.fixed_args.len() + 10);
        argv.push(self.scanner.program.clone());
        argv.extend(self.scanner.fixed_args.iter().cloned());
        argv.extend(build_scanner_argv(&params.target, ports.as_deref()));

        // (3) Chạy scanner trong sandbox riêng (có mạng — xem `ScanSandboxConfig`).
        let timeout = std::time::Duration::from_secs(self.sandbox.timeout_seconds());
        let outcome = self
            .sandbox
            .run_argv(&argv, timeout, &ctx.cancel)
            .await
            .map_err(scan_error)?;

        let mut output = outcome.stdout;
        if !outcome.stderr.is_empty() {
            output.push_str("\n--- stderr ---\n");
            output.push_str(&outcome.stderr);
        }
        let label = self.scope.label_for(&params.target).unwrap_or_default();
        let report = build_report(&params.target, label, &output);

        // (4) Cảnh báo mức cao gửi THẲNG cho kênh chính, song song với báo cáo chuẩn hoá
        //     (M23). Chỉ mức `High`; lỗi gửi chỉ ghi log, không làm hỏng tool (mục 6).
        if report.severity.needs_direct_alert()
            && let Some(sink) = ctx.alerts.as_ref()
        {
            let alert = Alert {
                severity: report.severity,
                title: "Cảnh báo quét bảo mật".to_string(),
                summary: report.summary.clone(),
                risks: report.risks.clone(),
            };
            if let Err(error) = sink.send_alert(&alert).await {
                tracing::warn!(error = %error, "gửi cảnh báo quét bảo mật thất bại");
            }
        }

        let rendered = format!("{}\n{output}", render_json(&report));
        Ok(wrap(&rendered))
    }
}

/// Bọc untrusted với trần cứng (thẻ đóng luôn còn, mục 15.4).
fn wrap(content: &str) -> String {
    beanagent_tools::wrap_bounded(content, beanagent_tools::MAX_WRAPPED_OUTPUT_CHARS)
}

/// Serialize báo cáo; struct phẳng này không thất bại nhưng vẫn có fallback (mục 0.8).
fn render_json(report: &ScanReport) -> String {
    serde_json::to_string(report)
        .unwrap_or_else(|_| r#"{"status":"ok","severity":"low"}"#.to_string())
}

/// Chuyển lỗi sandbox thành `ToolError`, không lộ argv (có thể chứa target nội bộ).
fn scan_error(error: SandboxError) -> ToolError {
    let message = match error {
        SandboxError::Timeout(seconds) => {
            format!("quét hết thời gian cho phép ({seconds}s) — đã bị kill")
        }
        SandboxError::Cancelled => "quét bị người dùng huỷ".to_string(),
        SandboxError::Launch(detail) => format!("không chạy được scanner: {detail}"),
    };
    ToolError::Io(beanagent_tools::wrap_untrusted(&message))
}
