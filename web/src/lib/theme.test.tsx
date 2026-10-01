import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it } from "vitest";

import { ThemeToggle } from "@/components/ThemeToggle";
import { I18nProvider } from "@/i18n";
import { DEFAULT_THEME, THEME_STORAGE_KEY, ThemeProvider } from "@/lib/theme";

/**
 * Chủ đề là thứ người dùng nhìn thấy đầu tiên và chạm vào đầu tiên, nên nó
 * được test riêng thay vì để sót. Ba lỗi dễ mắc, mỗi lỗi một test:
 *   1. quên bơm class lên `<html>`  → CSS không đổi, mọi thứ vẫn sáng
 *   2. quên lưu lại               → tải lại trang thì mất lựa chọn
 *   3. mặc định không phải tối    → lệch với diện mạo sản phẩm
 */
function renderToggle() {
  return render(<ThemeToggle />, { wrapper: Wrapper });
}

/** Provider dùng chung cho các test render `ThemeToggle` trực tiếp. */
function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ThemeProvider>
      <I18nProvider>{children}</I18nProvider>
    </ThemeProvider>
  );
}

describe("ThemeToggle", () => {
  beforeEach(() => {
    document.documentElement.className = "";
    window.localStorage.clear();
  });

  it("mặc định là tối và bơm class dark lên <html>", () => {
    expect(DEFAULT_THEME).toBe("dark");
    renderToggle();
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.classList.contains("light")).toBe(false);
  });

  it("đọc lựa chọn đã lưu và khôi phục đúng chủ đề", () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, "light");
    renderToggle();
    expect(document.documentElement.classList.contains("light")).toBe(true);
  });

  it("bấm nút thì đổi chủ đề, lưu lại và đổi nhãn", async () => {
    const user = userEvent.setup();
    renderToggle();

    const button = screen.getByRole("button");
    expect(button).toHaveAttribute("aria-pressed", "true");

    await user.click(button);

    expect(document.documentElement.classList.contains("light")).toBe(true);
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");
    // Nhãn phải mô tả hành động SẮP thực hiện, không phải trạng thái hiện tại.
    expect(button).toHaveAttribute("aria-pressed", "false");
    expect(button.getAttribute("aria-label")).toBeTruthy();
  });

  // `label` là chữ cạnh icon. Nó chỉ nói **đây là gì**, không nói **đang là
  // gì** — icon trơn không đủ để biết chủ đề hiện tại mà không bấm thử. Vì vậy nút
  // phải hiện tên trạng thái, và nó phải đổi theo icon.
  it("có label thì hiện tên trạng thái, và đổi khi bấm", async () => {
    const user = userEvent.setup();
    render(<ThemeToggle label="Giao diện" />, { wrapper: Wrapper });

    const button = screen.getByRole("button");
    expect(button).toHaveTextContent("Giao diện");
    expect(button).toHaveTextContent("Tối");
    expect(button).toHaveAttribute("data-theme-state", "dark");

    await user.click(button);

    expect(button).toHaveTextContent("Sáng");
    expect(button).toHaveAttribute("data-theme-state", "light");
  });

  // Chữ trên nút phải nằm trong tên trợ năng, nếu không trình đọc màn hình đọc
  // câu hành động mà bỏ mất phần "đang là gì" đang hiện trên màn hình.
  it("aria-label chứa cả nhãn lẫn trạng thái đang áp dụng", () => {
    render(<ThemeToggle label="Giao diện" />, { wrapper: Wrapper });
    const name = screen.getByRole("button").getAttribute("aria-label") ?? "";
    expect(name).toContain("Giao diện");
    expect(name).toContain("Tối");
    expect(name).toContain("Chuyển sang giao diện sáng");
  });

  // Không `label` thì chỉ còn icon — dùng ở chỗ hẹp (rail 80px) không chứa nổi
  // chữ. `aria-label` vẫn phải đủ để dùng bằng trình đọc màn hình.
  it("không label thì không có chữ, nhưng aria-label vẫn đầy đủ", () => {
    render(<ThemeToggle />, { wrapper: Wrapper });
    const button = screen.getByRole("button");
    expect(button.textContent).toBe("");
    expect(button).toHaveAccessibleName("Chuyển sang giao diện sáng");
  });
});
