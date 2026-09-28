//! `ScanScope` — kiểm tra target có nằm trong `[[infra_scope]]` hay không (M23).
//!
//! # Vì sao đây là thứ quan trọng nhất của M23
//!
//! Agent có quyền chạy lệnh. Nếu model bị prompt injection (qua log, ticket, file…) bảo quét
//! một host tấn công bất kỳ, máy chủ của bạn sẽ trở thành **vũ khí tấn công do chính bạn vận
//! hành**. Vì vậy mọi lần quét đều phải đi qua đây **trước khi** spawn scanner, và phải kiểm ở
//! **tầng code** — không được dựa vào model tự kiểm tra (`Plan.md` M23).
//!
//! # Vì sao không dùng crate `ipnet`
//!
//! So khớp CIDR chỉ cần vài dòng với `IpAddr`; thêm dependency vào một công cụ bảo mật để
//! tránh 20 dòng tự viết là đánh đổi xấu (ràng buộc chuỗi cung ứng, `agents.md` mục 15.10).
//!
//! # Vì sao không nhận hostname
//!
//! Code kiểm scope và scanner sẽ phân giải DNS ở **hai thời điểm khác nhau**. Kẻ tấn công điều
//! khiển DNS có thể trả về IP được phép lúc kiểm tra, rồi đổi sang IP khác lúc scanner chạy
//! (khe hở TOCTOU). Vì vậy scope chỉ nhận `ip`/`cidr` — xem `ScanTargetKind`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use bean_types::config::{ScanScopeEntry, ScanTargetKind};

/// Một dải CIDR đã dịch sang dạng so sánh được.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cidr {
    V4 { base: Ipv4Addr, prefix: u8 },
    V6 { base: Ipv6Addr, prefix: u8 },
}

impl Cidr {
    /// Dựng từ chuỗi `addr/len`; `None` nếu cú pháp sai (phần validate của `Config` đã chặn
    /// trước, đây là lưới an toàn thứ hai).
    fn parse(value: &str) -> Option<Self> {
        let (addr, len) = value.split_once('/')?;
        let prefix: u8 = len.parse().ok()?;
        match addr.parse::<IpAddr>().ok()? {
            IpAddr::V4(v4) => (prefix <= 32).then_some(Self::V4 { base: v4, prefix }),
            IpAddr::V6(v6) => (prefix <= 128).then_some(Self::V6 { base: v6, prefix }),
        }
    }

    /// `ip` có nằm trong dải này không?
    fn contains(&self, ip: &IpAddr) -> bool {
        match (self, ip) {
            (Self::V4 { base, prefix }, IpAddr::V4(candidate)) => {
                mask_v4(*base, *prefix, *candidate)
            }
            (Self::V6 { base, prefix }, IpAddr::V6(candidate)) => {
                mask_v6(*base, *prefix, *candidate)
            }
            // Khác họ địa chỉ ⇒ chắc chắn không khớp (không bao giờ "nới" để cho qua).
            _ => false,
        }
    }
}

/// So khớp IPv4 sau khi áp mask tiền tố.
fn mask_v4(base: Ipv4Addr, prefix: u8, candidate: Ipv4Addr) -> bool {
    let mask: u32 = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    };
    u32::from(base) & mask == u32::from(candidate) & mask
}

/// So khớp IPv6 sau khi áp mask tiền tố.
fn mask_v6(base: Ipv6Addr, prefix: u8, candidate: Ipv6Addr) -> bool {
    let base = u128::from(base);
    let candidate = u128::from(candidate);
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix as u32)
    };
    base & mask == candidate & mask
}

/// Một entry của scope kèm nhãn để báo cáo nói rõ đang quét cái gì.
#[derive(Debug, Clone)]
struct ScopeItem {
    range: Cidr,
    label: String,
}

/// Danh sách target được phép quét.
///
/// **Rỗng = từ chối mọi thứ** (fail-closed). Đây là mặc định an toàn của M23: mở tính năng
/// mà chưa khai báo scope thì agent không thể quét được bất kỳ host nào.
#[derive(Debug, Clone, Default)]
pub struct ScanScope {
    items: Vec<ScopeItem>,
    exact: Vec<(IpAddr, String)>,
}

impl ScanScope {
    /// Dựng scope từ cấu hình; entry sai cú pháp bị **bỏ qua** và ghi cảnh báo
    /// (`Config::validate` đã chặn trước đó, nên nhánh này chỉ là lưới an toàn).
    #[must_use]
    pub fn from_config(entries: &[ScanScopeEntry]) -> Self {
        let mut scope = Self::default();
        for entry in entries {
            let value = entry.value.trim();
            let label = entry.label.trim().to_string();
            match entry.kind {
                ScanTargetKind::Cidr => match Cidr::parse(value) {
                    Some(range) => scope.items.push(ScopeItem { range, label }),
                    None => tracing::warn!(
                        target = value,
                        "[[infra_scope]] bỏ qua CIDR không hợp lệ — mọi lần quét tới target này sẽ bị từ chối"
                    ),
                },
                ScanTargetKind::Ip => match value.parse::<IpAddr>() {
                    Ok(ip) => scope.exact.push((ip, label)),
                    Err(_) => tracing::warn!(
                        target = value,
                        "[[infra_scope]] bỏ qua IP không hợp lệ — mọi lần quét tới target này sẽ bị từ chối"
                    ),
                },
            }
        }
        scope
    }

    /// Scope có rỗng không? (rỗng ⇒ mọi lần quét đều bị từ chối)
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.exact.is_empty()
    }

    /// `target` có được phép quét không?
    ///
    /// Chỉ nhận **IP viết dạng chữ**; hostname bị từ chối vì lý do TOCTOU (xem module docs).
    #[must_use]
    pub fn allows(&self, target: &str) -> bool {
        let Ok(ip) = parse_literal_ip(target) else {
            return false;
        };
        self.exact.iter().any(|(allowed, _)| *allowed == ip)
            || self.items.iter().any(|item| item.range.contains(&ip))
    }

    /// Nhãn của entry khớp `target` (phục vụ báo cáo), hoặc `None` nếu ngoài scope.
    #[must_use]
    pub fn label_for(&self, target: &str) -> Option<&str> {
        let ip = parse_literal_ip(target).ok()?;
        self.exact
            .iter()
            .find(|(allowed, _)| *allowed == ip)
            .map(|(_, label)| label.as_str())
            .or_else(|| {
                self.items
                    .iter()
                    .find(|item| item.range.contains(&ip))
                    .map(|item| item.label.as_str())
            })
    }
}

/// Parse IP **dạng chữ**; từ chối mọi thứ trông giống hostname để không bao giờ gửi tên miền
/// cho scanner.
fn parse_literal_ip(target: &str) -> Result<IpAddr, ()> {
    if target.is_empty()
        || target.len() > 45
        || !target
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b':' || b.is_ascii_hexdigit())
    {
        return Err(());
    }
    target.parse::<IpAddr>().map_err(|_| ())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{Cidr, ScanScope};
    use bean_types::config::{ScanScopeEntry, ScanTargetKind};
    use std::net::IpAddr;

    fn entry(kind: ScanTargetKind, value: &str, label: &str) -> ScanScopeEntry {
        ScanScopeEntry {
            kind,
            value: value.to_string(),
            label: label.to_string(),
        }
    }

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    /// M23 yêu cầu: target ngoài scope bị từ chối — và scope rỗng phải chặn **mọi thứ**.
    #[test]
    fn empty_scope_denies_everything() {
        let scope = ScanScope::default();
        assert!(scope.is_empty());
        for target in ["203.0.113.7", "192.168.10.5", "8.8.8.8", "::1"] {
            assert!(!scope.allows(target), "scope rỗng phải chặn {target}");
        }
    }

    #[test]
    fn cidr_and_exact_ip_are_matched() {
        let scope = ScanScope::from_config(&[
            entry(ScanTargetKind::Cidr, "192.168.10.0/24", "VLAN IT"),
            entry(ScanTargetKind::Ip, "203.0.113.7", "VPN gateway"),
        ]);
        assert!(scope.allows("192.168.10.5"));
        assert!(scope.allows("192.168.10.255"));
        assert!(scope.allows("203.0.113.7"));
        assert!(!scope.allows("192.168.11.5"), "ngoài dải /24");
        assert!(!scope.allows("203.0.113.8"), "khác IP lẻ");
        assert!(!scope.allows("8.8.8.8"), "ngoài scope hoàn toàn");
        assert_eq!(scope.label_for("192.168.10.5"), Some("VLAN IT"));
        assert_eq!(scope.label_for("8.8.8.8"), None);
    }

    /// Hostname và payload shell **không bao giờ** được coi là target hợp lệ.
    #[test]
    fn hostnames_and_injection_payloads_are_always_rejected() {
        let scope = ScanScope::from_config(&[entry(ScanTargetKind::Cidr, "10.0.0.0/8", "nội bộ")]);
        for target in [
            "example.com",
            "localhost",
            "10.0.0.1; rm -rf /",
            "1.1.1.1 && curl evil.test",
            "",
            " 10.0.0.1",
            "10.0.0.1\n10.0.0.2",
        ] {
            assert!(
                !scope.allows(target),
                "hostname/payload phải bị chặn: {target:?}"
            );
        }
    }

    #[test]
    fn prefix_edges_behave_and_invalid_is_rejected() {
        assert!(Cidr::parse("0.0.0.0/0").is_some_and(|c| c.contains(&ip("8.8.8.8"))));
        assert!(Cidr::parse("1.2.3.4/32").is_some_and(|c| c.contains(&ip("1.2.3.4"))));
        assert!(!Cidr::parse("1.2.3.4/32").is_some_and(|c| c.contains(&ip("1.2.3.5"))));
        assert!(Cidr::parse("1.2.3.0/33").is_none(), "/33 là sai");
        assert!(Cidr::parse("1.2.3.0").is_none(), "thiếu /len");
    }

    /// IPv6 trong scope không bao giờ "rơi" xuống khớp IPv4 và ngược lại.
    #[test]
    fn address_families_never_cross_match() {
        let scope = ScanScope::from_config(&[entry(ScanTargetKind::Cidr, "fd00::/8", "v6")]);
        assert!(scope.allows("fd00::1"));
        assert!(!scope.allows("192.168.0.1"), "khác họ địa chỉ phải bị chặn");
    }

    /// Entry hỏng phải bị **bỏ qua**, tuyệt đối không nới scope thành "cho phép tất cả".
    #[test]
    fn invalid_entry_is_skipped_not_widened() {
        let scope = ScanScope::from_config(&[entry(ScanTargetKind::Cidr, "khong-phai-cidr", "x")]);
        assert!(
            scope.is_empty(),
            "entry hỏng phải bị bỏ qua, không nới scope"
        );
        assert!(!scope.allows("192.168.1.1"));
    }
}
