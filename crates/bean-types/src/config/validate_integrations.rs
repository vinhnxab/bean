//! Kiểm tra cấu hình tích hợp: mcp, billing, browser, marketing, infra, qa.

use std::collections::BTreeSet;

use crate::rbac::WILDCARD_TAG;

use super::error::invalid;
use super::validate::validate_http_url;
use super::validate::validate_inherited_env_name;
use super::validate::validate_origin_pattern;
use super::*;

impl Config {
    /// Kiểm tra hai trường mới của `[[mcp_servers]]` — phía Bean là MCP **client**
    /// (mục 16): `call_timeout_seconds` và `inherit_env`.
    ///
    /// Không lặp lại kiểm tra `name`/`command` — [`Self::validate_web_and_channels`]
    /// đã làm việc đó (và với thông điệp riêng). Ở đây chỉ những gì **chỉ** trường mới
    /// mới làm hỏng được.
    ///
    /// Chặn ở **tầng config** (nguyên tắc đã dùng cho `forbid_tags` M21.6) vì cả hai
    /// lỗi dưới đây đều im lặng nếu để tới tầng spawn:
    ///
    /// * `call_timeout_seconds = 0` ⇒ `Duration::ZERO` ⇒ **mọi** tool call bị treo tức
    ///   thì và trả về lỗi, model sẽ tưởng server hỏng rồi thử lại vô tận;
    /// * tên biến sai trong `inherit_env` ⇒ `Command::env` **panic** — tệ hơn lỗi
    ///   (`unwrap` trên dữ liệu bên ngoài, mục 22.11).
    pub(super) fn validate_mcp_servers(&self) -> Result<(), ConfigError> {
        for server in &self.mcp_servers {
            if let Some(seconds) = server.call_timeout_seconds
                && !(1..=MAX_MCP_CALL_TIMEOUT_SECONDS).contains(&seconds)
            {
                return Err(invalid(format!(
                    "[[mcp_servers]].call_timeout_seconds của `{}` = {seconds} nằm ngoài 1..={MAX_MCP_CALL_TIMEOUT_SECONDS}; \
                     0 sẽ khiến mọi tool call bị treo tức thì",
                    server.name
                )));
            }
            for variable in &server.inherit_env {
                validate_inherited_env_name(&server.name, variable)?;
            }
        }
        Ok(())
    }
}
impl Config {
    /// Kiểm tra `[[mcp_clients]]` + `[mcp_server]` (M25).
    ///
    /// Chặn ở **tầng config** những cấu hình làm MCP server vô nghĩa hoặc không an toàn, để
    /// không phải "phát hiện khi model tự gọi" (nguyên tắc đã dùng cho `forbid_tags` M21.6):
    ///
    /// * client phải có identity `mcp-client:<name>` trong `agent.user_roles` và role đó
    ///   phải tồn tại — nếu không, `permissions_for()` trả `no-access` và client chỉ
    ///   thấy danh sách rỗng mà không ai hiểu vì sao;
    /// * client **không** được dùng role `admin` (tag `*`) — vai trò "mọi quyền" không có
    ///   nghĩa trên bề mặt MCP read-only, và cho phép nó sẽ khiến người đọc cấu hình tưởng
    ///   rằng client có thể ghi/thực thi (M25 phạm vi cứng);
    /// * HTTP bật mà bind ngoài loopback thì phải `allow_remote = true`.
    pub(super) fn validate_mcp_server(&self) -> Result<(), ConfigError> {
        let mut seen = BTreeSet::new();
        for client in &self.mcp_clients {
            let name = client.name.trim();
            if name.is_empty() {
                return Err(invalid("[[mcp_clients]] có client thiếu tên"));
            }
            if !seen.insert(name.to_string()) {
                return Err(invalid(format!(
                    "[[mcp_clients]] có client trùng tên `{name}`"
                )));
            }
            if name.contains('/') || name.contains('\\') || name.contains(char::is_whitespace) {
                return Err(invalid(format!(
                    "[[mcp_clients]].name = `{name}` chứa ký tự không hợp lệ (kebab-case, không khoảng trắng hay dấu gạch chéo)"
                )));
            }
            if self.role(&client.role).is_none() {
                return Err(invalid(format!(
                    "[[mcp_clients]].role = `{}` của `{name}` không tồn tại trong [[roles]]",
                    client.role
                )));
            }
            if self
                .role(&client.role)
                .is_some_and(|role| role.tag_set().contains(WILDCARD_TAG))
            {
                return Err(invalid(format!(
                    "[[mcp_clients]].role = `{}` của `{name}` giữ tag `*` (admin) — MCP server \
                     M25 chỉ read-only, cấp mọi quyền ở đây chỉ gây hiểu nhầm",
                    client.role
                )));
            }
            let identity = format!("{MCP_CLIENT_PREFIX}{name}");
            match self.agent.user_roles.get(&identity) {
                None => {
                    return Err(invalid(format!(
                        "thiếu `{identity}` trong agent.user_roles — client MCP `{name}` sẽ \
                         resolve thành no-access và không thấy tool nào"
                    )));
                }
                Some(mapped) if mapped != &client.role => {
                    return Err(invalid(format!(
                        "agent.user_roles[`{identity}`] = `{mapped}` khác [[mcp_clients]].role = `{}`",
                        client.role
                    )));
                }
                Some(_) => {}
            }
        }
        if self.mcp_server.http_enabled
            && !self.mcp_server.bind.ip().is_loopback()
            && !self.mcp_server.allow_remote
        {
            return Err(invalid(
                "[mcp_server].bind ngoài loopback yêu cầu allow_remote = true; nếu public, phải đặt sau reverse proxy TLS/Tailscale/VPN",
            ));
        }
        if self.mcp_server.enabled && self.mcp_clients.is_empty() {
            return Err(invalid(
                "[mcp_server].enabled = true nhưng [[mcp_clients]] rỗng — không client nào được xác thực; \
                 chạy `bean auth mcp-token add <tên>` để sinh token",
            ));
        }
        // K24: bề mặt HTTP có thể công khai nên phải có trần tần suất. Cấu hình sai
        // (bật HTTP mà đặt 0) phải chết lúc nạp chứ không âm thầm mở đường không giới hạn.
        if self.mcp_server.http_enabled && self.mcp_server.rate_limit_per_minute == 0 {
            return Err(invalid(
                "[mcp_server].http_enabled = true mà rate_limit_per_minute = 0 — transport HTTP là \
                 bề mặt có thể công khai, cần trần tần suất để chặn dò token; đặt 0 chỉ hợp lệ \
                 khi bạn tự chịu trách nhiệm (ví dụ chỉ bind loopback)",
            ));
        }
        Ok(())
    }
}
impl Config {
    /// Kiểm tra `[billing]` (M22a).
    ///
    /// Trọng tâm là **tách credential**: `api_key_env` của billing phải là biến riêng, không
    /// được trùng với biến của LLM / web search / Telegram. Đây là cách hiện thực hoá yêu cầu
    /// "credential đọc billing phải riêng, quyền tối thiểu chỉ đọc billing" ở tầng code thay
    /// vì chỉ dựa vào quy ước viết trong tài liệu (D13.2).
    pub(super) fn validate_billing(&self) -> Result<(), ConfigError> {
        if !self.billing.enabled {
            return Ok(());
        }
        if self.billing.api_key_env.trim().is_empty() {
            return Err(invalid("[billing].api_key_env rỗng"));
        }
        // Endpoint chỉ lấy từ cấu hình, nhưng vẫn chặn scheme lạ để không biến `base_url`
        // thành đường đọc file cục bộ nếu cấu hình bị sửa nhầm.
        if let Some(base) = &self.billing.base_url {
            validate_http_url("[billing].base_url", base)?;
        }
        if let Some(suffix) = &self.billing.query_suffix
            && (suffix.contains("://") || suffix.contains('@'))
        {
            return Err(invalid(
                "[billing].query_suffix phải chỉ là tham số truy vấn (vd `?period=30d`), không phải URL",
            ));
        }
        let others: [(&str, &str); 3] = [
            ("llm.api_key_env", &self.llm.api_key_env),
            (
                "tools.web_search.api_key_env",
                &self.tools.web_search.api_key_env,
            ),
            ("telegram.token_env", &self.telegram.token_env),
        ];
        for (field, env) in others {
            if env.trim() == self.billing.api_key_env.trim() {
                return Err(invalid(format!(
                    "[billing].api_key_env không được trùng với {field} (`{env}`): credential đọc billing phải RIÊNG, quyền tối thiểu chỉ đọc billing"
                )));
            }
        }
        Ok(())
    }
}
impl Config {
    /// Kiểm tra `[browser]` (M26).
    ///
    /// Chặn ở **tầng config** (cùng nguyên tắc `forbid_tags` M21.6) vì ba lỗi dưới
    /// đây đều **im lặng** hoặc nguy hiểm nếu để tới tầng chạy:
    ///
    /// * mẫu origin sai cú pháp ⇒ bị bỏ qua lúc parse ⇒ người dùng tưởng đã mở
    ///   whitelist mà thật ra mọi origin vẫn `Dangerous` (lỗi "thắt chặt", ít nguy
    ///   hiểm hơn, nhưng im lặng thì không ai biết phải sửa);
    /// * wildcard ở label không đầu tiên (`https://dev.*.internal`) ⇒ **không** có
    ///   nghĩa an toàn nào, và dễ khiến người dùng tưởng nó hoạt động;
    /// * `max_image_bytes` bằng 0 ⇒ **mọi** lần chụp đều thất bại, model không hiểu
    ///   vì sao.
    pub(super) fn validate_browser(&self) -> Result<(), ConfigError> {
        let browser = &self.browser;
        if !browser.enabled {
            // Tắt thì không cần validate whitelist: người dùng có thể để sẵn mẫu
            // đang viết rồi bật sau. Việc sửa đường dẫn Chrome cũng vậy.
            return Ok(());
        }
        for pattern in &browser.allowed_origins {
            if let Err(reason) = validate_origin_pattern(pattern) {
                return Err(invalid(format!(
                    "[browser].allowed_origins chứa mẫu không hợp lệ `{pattern}`: {reason}"
                )));
            }
        }
        if browser.executable_path.chars().count() > 4_096 {
            return Err(invalid(
                "[browser].executable_path quá dài; đây là đường dẫn, không phải nội dung",
            ));
        }
        if browser.max_image_bytes == 0 {
            return Err(invalid(
                "[browser].max_image_bytes = 0 khiến mọi lần chụp đều thất bại; \
                 đặt trần thực tế (mặc định 4 MiB)",
            ));
        }
        if browser.launch_timeout_seconds == 0 {
            return Err(invalid(
                "[browser].launch_timeout_seconds = 0 khiến Chrome không có thời gian \
                 khởi động",
            ));
        }
        Ok(())
    }
}
impl Config {
    /// Kiểm tra `[marketing]` (M24).
    pub(super) fn validate_marketing(&self) -> Result<(), ConfigError> {
        let env = self.marketing.api_key_env.trim();
        if env.is_empty() {
            return Err(invalid("[marketing].api_key_env không được để trống"));
        }
        // (D15.3) Credential phải RIÊNG: quyền "chỉ post" không được dùng lại key có
        // quyền rộng hơn (LLM, search, Telegram). Cùng mẫu với M22a — kiểm ở tầng load
        // config, không dựa vào kỷ luật vận hành.
        for (other, what) in [
            (self.llm.api_key_env.trim(), "llm.api_key_env"),
            (
                self.tools.web_search.api_key_env.trim(),
                "tools.web_search.api_key_env",
            ),
            (self.telegram.token_env.trim(), "telegram.token_env"),
        ] {
            if !other.is_empty() && other == env {
                return Err(invalid(format!(
                    "[marketing].api_key_env trùng với {what} — M24 yêu cầu credential riêng, \
                     quyền tối thiểu chỉ post"
                )));
            }
        }
        if self.marketing.text_field.trim().is_empty() {
            return Err(invalid("[marketing].text_field không được để trống"));
        }
        Ok(())
    }
}
impl Config {
    /// Kiểm tra `[[infra_scope]]` + `[security_scan]` (M23).
    ///
    /// `infra_scope` rỗng **hợp lệ** và nghĩa là "từ chối mọi thứ" (fail-closed), nên
    /// `validate()` không yêu cầu phải có scope.
    pub(super) fn validate_infra_scope(&self) -> Result<(), ConfigError> {
        let mut seen = BTreeSet::new();
        for entry in &self.infra_scope {
            let value = entry.value.trim();
            if !seen.insert(value.to_string()) {
                return Err(invalid(format!(
                    "[[infra_scope]] có target trùng lặp `{value}`"
                )));
            }
            if let Err(reason) = validate_scope_value(entry.kind, value) {
                return Err(invalid(format!(
                    "[[infra_scope]] target `{value}` không hợp lệ: {reason}. Chỉ chấp nhận IP (vd 203.0.113.7) hoặc CIDR (vd 192.168.10.0/24), KHÔNG nhận hostname"
                )));
            }
        }

        // (D14.3) Scanner bắt buộc cần mạng: cấm tắt để không tạo cấu hình "bật nhưng vô dụng".
        if !self.security_scan.sandbox.network {
            return Err(invalid(
                "[security_scan].sandbox.network phải là true — scanner không có mạng thì không tới được target trong scope",
            ));
        }
        // (D14.4) Chế độ host phải được bật tường minh, tránh vô tình hạ cấp cách ly.
        if self.security_scan.enabled
            && self.security_scan.sandbox.mode == SandboxMode::Host
            && !self.security_scan.sandbox.allow_host
        {
            return Err(invalid(
                "[security_scan].sandbox.mode = \"host\" cần đặt allow_host = true để xác nhận bạn chấp nhận chạy scanner ngoài container",
            ));
        }
        if self.security_scan.sandbox.timeout_seconds == 0 {
            return Err(invalid("[security_scan].sandbox.timeout_seconds phải > 0"));
        }
        // Cảnh báo cần đủ cặp channel + chat_id, nếu không thì cảnh báo im lặng (rất dễ quên).
        if self.security_scan.enabled {
            let has_channel = !self.security_scan.alert_channel.trim().is_empty();
            let has_chat = !self.security_scan.alert_chat_id.trim().is_empty();
            if has_channel != has_chat {
                return Err(invalid(
                    "[security_scan].alert_channel và alert_chat_id phải khai báo cùng nhau (hoặc cả hai để trống = không gửi cảnh báo)",
                ));
            }
        }
        Ok(())
    }
}
impl Config {
    /// Kiểm tra `[qa]` + `[[qa.suites]]` (M27).
    ///
    /// `suites` rỗng **hợp lệ** và nghĩa là "từ chối mọi thứ" (fail-closed, y hệt
    /// `[[infra_scope]]` rỗng của M23) — bật `[qa]` mà chưa khai suite thì chỉ đăng ký
    /// được tool mà không chạy được suite nào.
    pub(super) fn validate_qa(&mut self) -> Result<(), ConfigError> {
        if self.qa.sandbox.timeout_seconds == 0 {
            return Err(invalid("[qa].sandbox.timeout_seconds phải > 0"));
        }
        if self.qa.sandbox.image.trim().is_empty() {
            return Err(invalid("[qa].sandbox.image không được để trống"));
        }

        let mut seen = BTreeSet::new();
        for suite in &self.qa.suites {
            let name = suite.name.trim();
            if name.is_empty() {
                return Err(invalid("[[qa.suites]] có suite thiếu tên"));
            }
            if !seen.insert(name.to_string()) {
                return Err(invalid(format!(
                    "[[qa.suites]] có suite trùng tên `{name}`"
                )));
            }
            // Tên suite đi vào argv và vào mô tả cho model ⇒ giới hạn bộ ký an toàn để
            // không thể tạo ra giá trị mơ hồ khi báo cáo.
            if name.len() > 64
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                return Err(invalid(format!(
                    "[[qa.suites]] tên `{name}` không hợp lệ — chỉ chấp nhận chữ/số và `-`, `_`, `.` (tối đa 64 ký tự)"
                )));
            }
            // `workdir` phải là đường dẫn **tương đối** trong workspace. Rỗng = gốc
            // workspace, hợp lệ. Không nhận `..`/đường dẫn tuyệt đối: `run_argv_readonly`
            // cũng chặn lặp ở tầng cuối, nhưng chặn sớm ở đây để lỗi lộ ra lúc nạp config
            // chứ không phải giữa lúc agent đang review.
            let workdir = suite.workdir.trim();
            // Rỗng = gốc workspace, hợp lệ — phải kiểm TRƯỚC khi tách, vì `"".split('/')`
            // trả về `[""]` và nhánh `part.is_empty()` bên dưới sẽ chặn nhầm.
            if !workdir.is_empty()
                && (workdir.starts_with('/')
                    || workdir
                        .split('/')
                        .any(|part| part == ".." || part.is_empty()))
            {
                return Err(invalid(format!(
                    "[[qa.suites]] `{name}` có workdir `{workdir}` không hợp lệ — phải là đường dẫn tương đối, không `..`, không bắt đầu bằng `/`"
                )));
            }
            if suite.args.iter().any(|arg| arg.trim().is_empty()) {
                return Err(invalid(format!(
                    "[[qa.suites]] `{name}` có tham số rỗng trong `args`"
                )));
            }
        }

        // (D17.2) Cờ mạng **không phải thứ cấu hình tự do**: `QaRunner::needs_network`
        // quyết định và `validate()` ghi đè giá trị khai báo. Ghi đè âm thầm sẽ khiến
        // người đọc file tưởng mình đang ở chế độ offline, nên phải cảnh báo.
        let wants_network = self
            .qa
            .suites
            .iter()
            .any(|suite| suite.runner.needs_network());
        if self.qa.sandbox.network != wants_network {
            tracing::warn!(
                configured = self.qa.sandbox.network,
                effective = wants_network,
                "[qa].sandbox.network bị ghi đè theo runner (D17.2): cargo_test cần mạng để \
                 tải crate, vitest/pytest chạy offline. Giá trị trong file không có tác dụng"
            );
            self.qa.sandbox.network = wants_network;
        }
        Ok(())
    }
}
/// Tên MCP server: chỉ chữ/số/`_`/`-` vì được ghép vào tên tool `mcp__<server>__<tool>`.
pub(super) fn is_valid_mcp_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}
