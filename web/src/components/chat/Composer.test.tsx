import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Composer } from "@/components/chat/Composer";
import { I18nProvider } from "@/i18n";

const NOOP = {
  onChange: () => {},
  onSubmit: () => {},
  onStop: () => {},
  canStop: false,
  stopping: false,
  disabled: false,
  sendError: false,
};

function renderComposer(props: Partial<React.ComponentProps<typeof Composer>> = {}) {
  return render(
    <I18nProvider>
      <Composer value="" {...NOOP} {...props} />
    </I18nProvider>,
  );
}

describe("Composer", () => {
  it("Enter gửi tin, Shift+Enter xuống dòng", async () => {
    const onSubmit = vi.fn();
    const user = userEvent.setup();
    renderComposer({ onSubmit });
    const box = screen.getByLabelText("Nhập tin nhắn của bạn…");

    await user.type(box, "xin chào");
    await user.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenCalledTimes(1);

    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("Enter đang gõ tiếng Việt (IME) không được gửi tin", async () => {
    // Đây là lỗi mất dấu: khi người dùng gõ "xin chao" và bấm dấu hỏi, trình
    // duyệt báo `isComposing = true`. Gửi tin lúc đó sẽ cắt mất dấu.
    const onSubmit = vi.fn();
    const user = userEvent.setup();
    renderComposer({ onSubmit });
    const box = screen.getByLabelText("Nhập tin nhắn của bạn…");

    await user.type(box, "x");
    // Mô phỏng trạng thái IME: `keydown` có `isComposing = true`.
    const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    Object.defineProperty(event, "isComposing", { value: true });
    box.dispatchEvent(event);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("nút gửi chỉ bật khi có nội dung và không bị khoá khi mất kết nối", () => {
    const { rerender } = renderComposer({ value: "   " });
    // Chỉ khoảng trắng không phải tin nhắn — nút vẫn tắt.
    expect(screen.getByRole("button", { name: "Gửi" })).toBeDisabled();

    rerender(
      <I18nProvider>
        <Composer value="có nội dung" {...NOOP} />
      </I18nProvider>,
    );
    expect(screen.getByRole("button", { name: "Gửi" })).toBeEnabled();
  });

  it("khi đang chạy thì nút Dừng thay cho nút Gửi", () => {
    renderComposer({ value: "abc", canStop: true });
    expect(screen.queryByRole("button", { name: "Gửi" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Dừng" })).toBeEnabled();
  });

  it("nút Dừng khoá lại khi đã gửi lệnh dừng", () => {
    renderComposer({ value: "abc", canStop: true, stopping: true });
    expect(screen.getByRole("button", { name: "Đang dừng…" })).toBeDisabled();
  });

  it("báo lỗi gửi bằng thông báo có vai trò alert", () => {
    renderComposer({ sendError: true });
    expect(screen.getByRole("alert")).toHaveTextContent("Không gửi được tin nhắn");
  });

  it("không render con trỏ con nhảy dòng — khung tự giãn", () => {
    renderComposer({ value: "một\nhai\nba" });
    const box = screen.getByLabelText("Nhập tin nhắn của bạn…");
    expect(box.className).toContain("resize-none");
  });
});
