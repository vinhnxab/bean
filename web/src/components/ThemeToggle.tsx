import { MoonIcon, SunIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n";
import { useTheme } from "@/lib/theme";

/**
 * Nút chuyển chủ đề tối/sáng.
 *
 * `aria-pressed` mô tả trạng thái hiện tại, nhãn nói rõ hành động sắp thực
 * hiện ("Chuyển sang giao diện sáng"), để người dùng đọc bằng màn hình đọc không
 * phải đoán từ biểu tượng.
 */
export function ThemeToggle({ className = "" }: { className?: string }) {
  const { theme, toggleTheme } = useTheme();
  const { t } = useI18n();
  const isDark = theme === "dark";

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      onClick={toggleTheme}
      aria-pressed={isDark}
      aria-label={isDark ? t("theme.toLight") : t("theme.toDark")}
      title={isDark ? t("theme.toLight") : t("theme.toDark")}
      data-slot="theme-toggle"
      className={className}
    >
      {isDark ? <MoonIcon /> : <SunIcon />}
    </Button>
  );
}
