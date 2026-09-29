import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
  return render(
    <ThemeProvider>
      <I18nProvider>
        <ThemeToggle />
      </I18nProvider>
    </ThemeProvider>,
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
});
