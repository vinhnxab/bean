import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useState } from "react";

/** Chủ đề giao diện. Bean **không** theo hệ thống — người dùng chọn, và tối là mặc định. */
export type Theme = "dark" | "light";

/** Mặc định là tối: nền đen + nâu đỏ là diện mạo chính của sản phẩm. */
export const DEFAULT_THEME: Theme = "dark";

/**
 * Khoá lưu lựa chọn. Phải **trùng** với hằng số trong script chống nháy trắng
 * của `index.html` — đó là bản chạy đồng bộ trước lần vẽ đầu tiên, còn
 * provider ở đây chạy sau khi React mount.
 */
export const THEME_STORAGE_KEY = "bean.theme";

function isTheme(value: unknown): value is Theme {
  return value === "dark" || value === "light";
}

/** Bơm class vào `<html>` — nơi duy nhất CSS `:root` / `.light` nhìn thấy. */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  root.classList.remove("dark", "light");
  root.classList.add(theme);
  // Trùng với `color-scheme` trong index.css, nhưng set ở đây giúp các
  // thành phần native (cuộn, form control, scrollbar) đổi màu ngay.
  root.style.colorScheme = theme;
}

type ThemeValue = {
  theme: Theme;
  setTheme: (theme: Theme) => void;
  toggleTheme: () => void;
};

const ThemeContext = createContext<ThemeValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  // Khởi tạo từ `localStorage` một cách **lười** (hàm khởi tạo) để lần render
  // đầu đã đúng chủ đề, không render tối rồi sáng rồi mới tối.
  const [theme, setThemeState] = useState<Theme>(() => {
    if (typeof window === "undefined") return DEFAULT_THEME;
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    return isTheme(stored) ? stored : DEFAULT_THEME;
  });

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    applyTheme(next);
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, next);
    } catch {
      // Chế độ riêng tư / storage bị chặn: vẫn đổi được trong phiên hiện tại,
      // chỉ mất lựa chọn sau khi tải lại. Không đáng báo lỗi ra giao diện.
    }
  }, []);

  // Đồng bộ `<html>` khi provider mount — phòng khi script chặn bị CSP loại
  // khỏi `index.html` (chúng ta không chèn script, nhưng test/harness thì có).
  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const value = useMemo<ThemeValue>(
    () => ({ theme, setTheme, toggleTheme: () => setTheme(theme === "dark" ? "light" : "dark") }),
    [theme, setTheme],
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeValue {
  const value = useContext(ThemeContext);
  if (!value) {
    throw new Error("useTheme phải được dùng bên trong <ThemeProvider>");
  }
  return value;
}
