/**
 * Chống nháy trắng khi đổi chủ đề (FOUC).
 *
 * File này chạy ĐỒNG BỘ trong <head>, trước lần vẽ đầu tiên, nên `<html>` đã
 * mang class đúng trước khi trình duyệt vẽ bất cứ thứ gì. Nếu để React lo
 * (`ThemeProvider` trong `src/lib/theme.tsx` chỉ chạy sau khi mount), người dùng
 * đã chọn chủ đề sáng sẽ nhìn thấy một nháy nền tối.
 *
 * Vì sao là file riêng mà không inline: CSP của server không khai báo
 * `script-src`, nên kế thừa `default-src 'self'` và chặn mọi inline script
 * (xem `crates/bean-web/src/server.rs`). Script thường trong <head> vẫn chặn
 * render nên vẫn bảo đảm chạy trước lần vẽ đầu.
 *
 * Hằng số ở đây phải khớp `THEME_STORAGE_KEY` và `DEFAULT_THEME` trong
 * `src/lib/theme.tsx` — tệp này không import được module ES.
 */
(function () {
  var root = document.documentElement;
  try {
    var stored = window.localStorage.getItem("bean.theme");
    var theme = stored === "light" || stored === "dark" ? stored : "dark";
    root.classList.remove("dark", "light");
    root.classList.add(theme);
    root.style.colorScheme = theme;
  } catch (err) {
    // Chế độ riêng tư / storage bị chặn: cứ để mặc định tối của `:root`.
    root.classList.add("dark");
  }
})();
