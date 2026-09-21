import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import App from "@/App";
import { I18nProvider } from "@/i18n";

/**
 * Test khói của M1: trang trống render được, dùng từ điển tiếng Việt mặc định
 * (agents.md mục 21 M1: "một test Vitest tối thiểu cho web").
 */
describe("App", () => {
  it("hiển thị tiêu đề và thông báo khung M1 bằng tiếng Việt", () => {
    render(
      <I18nProvider>
        <App />
      </I18nProvider>,
    );

    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("BeanAgent");
    expect(screen.getByText(/Trợ lý AI cá nhân/)).toBeInTheDocument();
    expect(screen.getByText(/Khung giao diện M1/)).toBeInTheDocument();
  });
});
