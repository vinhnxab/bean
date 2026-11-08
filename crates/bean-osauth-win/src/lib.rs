//! FFI Win32 `LogonUserW` — xác minh mật khẩu tài khoản Windows của tiến trình
//! Bean (D22.2: toggle host mode step-up bằng mật khẩu máy).
//!
//! # Vì sao tách crate riêng (miễn trừ `forbid(unsafe_code)`)
//!
//! Workspace đặt `unsafe_code = "forbid"` (AGENTS.md mục 0.8) và mọi crate khác
//! đều `#![forbid(unsafe_code)]`. Xác minh mật khẩu Windows **không có đường
//! Rust an toàn**: `LogonUserW` là API chuẩn duy nhất xác minh mật khẩu tài khoản
//! cục bộ mà không cần service riêng; shell-out qua PowerShell sẽ phải đưa mật
//! khẩu vào argv/env (lộ trong `ps`/WMI) — tệ hơn về bảo mật. Vậy nên unsafe được
//! **gói trong crate này duy nhất**, một hàm công khai, mọi call có chú thích
//! `# Safety`. Đã được người chủ dự án chấp nhận trong yêu cầu D22.2.
//!
//! # Hạn chế đã biết
//!
//! `LogonUserW` với `LOGON32_LOGON_INTERACTIVE` chạy được khi tiến trình thuộc
//! phiên đăng nhập tương tác của chính tài khoản đó (trường hợp `bean serve`
//! thông thường). Tài khoản bị политики khoá (AccountDisabled/DenyInteractive)
//! trả lỗi — caller phải map sang lỗi hạ tầng, không phải "sai mật khẩu".

/// Lỗi logon Windows. `InvalidCredentials` = sai mật khẩu (mã 1326/1327/1330);
/// còn lại là lỗi hạ tầng — caller không được tính vào bộ đếm khoá.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogonError {
    /// Mật khẩu sai (hoặc tài khoản không logon tương tác được).
    InvalidCredentials,
    /// Lỗi khác — mô tả kèm mã `GetLastError`.
    Failed(String),
}

/// Xác minh `password` cho tài khoản `username` trên máy cục bộ bằng
/// `LogonUserW(LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT)`.
///
/// Thành công ⇒ handle token được đóng ngay (không giữ token — mục đích chỉ là
/// xác minh, không mạo danh).
pub fn verify_logon(username: &str, password: &str) -> Result<(), LogonError> {
    let user_utf16 = to_wide_null(username);
    let pass_utf16 = to_wide_null(password);
    let domain = [0u16]; // NULL domain — xác minh trên máy cục bộ

    let mut token: windows_sys::Win32::Foundation::HANDLE = std::ptr::null_mut();
    // # Safety
    // * `user_utf16`/`pass_utf16`/`domain` đều là buffer UTF-16 kết thúc bằng null
    //   sống trong suốt lời gọi.
    // * `token` là pointer ghi được hợp lệ.
    // * Không giữ token sau khi đóng — chỉ dùng kết quả thành công/thất bại.
    let ok = unsafe {
        windows_sys::Win32::Security::LogonUserW(
            user_utf16.as_ptr(),
            domain.as_ptr(),
            pass_utf16.as_ptr(),
            windows_sys::Win32::Security::LOGON32_LOGON_INTERACTIVE,
            windows_sys::Win32::Security::LOGON32_PROVIDER_DEFAULT,
            &mut token,
        )
    };
    if ok != 0 {
        // # Safety
        // `token` vừa được LogonUserW trả về hợp lệ; CloseHandle đúng một lần.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(token) };
        return Ok(());
    }
    // # Safety
    // `GetLastError` không có điều kiện trước.
    let error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    match error {
        // ERROR_LOGON_FAILURE, ERROR_ACCOUNT_RESTRICTION, ERROR_INVALID_LOGON_HOURS,
        // ERROR_INVALID_WORKSTATION, ERROR_PASSWORD_EXPIRED, ERROR_ACCOUNT_DISABLED
        1326 | 1327 | 1328 | 1330 | 1331 | 1332 => Err(LogonError::InvalidCredentials),
        code => Err(LogonError::Failed(format!(
            "LogonUserW GetLastError={code}"
        ))),
    }
}

/// UTF-16 kết thúc null cho API Win32 wide-char.
fn to_wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_wide_null_appends_terminator() {
        assert_eq!(to_wide_null("ab"), vec![b'a' as u16, b'b' as u16, 0]);
        assert_eq!(to_wide_null(""), vec![0]);
    }

    // `verify_logon` chỉ gọi được trên Windows thật (link LogonUserW từ advapi32) —
    // không test gọi trên nền tảng khác; compose-only ở đây.
}
