import { MoonIcon, SunIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n";
import { useTheme } from "@/lib/theme";

/**
 * Nút chuyển chủ đề tối/sáng.
 *
 * # Vì sao hiện **cả** tên trạng thái lẫn hành động
 *
 * Icon trơn (`<MoonIcon />` / `<SunIcon />`) không nói được chủ đề đang bật: đọc
 * biểu tượng trăng phải biết quy ước là "tối" mới hiểu. Vì vậy khi có `label`,
 * nút hiện **`<label> · <tên trạng thái>`** — "Giao diện · Tối" — đọc là biết ngay
 * mà không phải bấm thử.
 *
 * Đây là điểm khác biệt so với `aria-pressed`: `aria-pressed` mang nghĩa
 * **bật/tắt**, nên trên nút chuyển đổi hai trạng thái nó luôn ở một trạng thái
 * nào đó và không cho biết "đang là gì". Ở đây người dùng cần **giá trị hiện
 * tại**, nên chữ trên nút làm việc đó; `aria-label` vẫn gộp cả hai để trình đọc
 * màn hình không bỏ sót phần nào.
 *
 * `label` mặc định **không có** — dùng ở chỗ không có nhãn (ví dụ trên điện
 * thoại) thì chỉ còn icon và `aria-label` như trước.
 */
export function ThemeToggle({ className = "", label }: { className?: string; label?: string }) {
  const { theme, toggleTheme } = useTheme();
  const { t } = useI18n();
  const isDark = theme === "dark";
  const action = isDark ? t("theme.toLight") : t("theme.toDark");
  // Tên trạng thái **đang áp dụng** (không phải sắp chuyển sang).
  const current = isDark ? t("theme.dark") : t("theme.light");

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      onClick={toggleTheme}
      aria-pressed={isDark}
      // `label` (chữ đang hiện) đứng đầu rồi mới tới giá trị và hành động — `WCAG
      // 2.5.3 Label in Name`: tên trợ năng phải **chứa** toàn bộ chữ nhìn thấy, nếu
      // không người đọc bằng trình đọc màn hình nghe câu không liên quan tới thứ
      // đang hiện trên màn hình.
      aria-label={label ? `${label}: ${current}, ${action}` : action}
      title={action}
      data-slot="theme-toggle"
      data-theme-state={theme}
      className={className}
    >
      {isDark ? <MoonIcon /> : <SunIcon />}
      {label ? (
        <>
          <span className="truncate">{label}</span>
          {/* `aria-hidden` vì đã nằm trong `aria-label`; đọc hai lần thì thành
              lặp — "Giao diện: Tối" rồi lại "Giao diện: Tối, Chuyển sang sáng". */}
          <span className="text-ink-muted" aria-hidden="true">
            ·
          </span>
          {/* Không `truncate`: giá trị là chữ ngắn ("Tối"), nếu bị cắt thì mất
              đúng thứ cần đọc. */}
          <span className="font-medium text-foreground">{current}</span>
        </>
      ) : null}
    </Button>
  );
}
