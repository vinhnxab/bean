import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MessageRow } from "@/components/chat/MessageRow";
import { I18nProvider } from "@/i18n";

function renderRow(ui: React.ReactNode) {
  return render(<I18nProvider>{ui}</I18nProvider>);
}

describe("MessageRow", () => {
  it("gắn tên người gửi và giờ để trình đọc màn hình đọc được ngữ cảnh", () => {
    renderRow(
      <MessageRow author="user" timestamp="2026-09-29T07:32:00Z" headingId="msg-1">
        Xin chào
      </MessageRow>,
    );
    // Nhãn lấy từ phần tử chứa tên + giờ, không phải từ toàn bộ nội dung tin.
    const region = screen.getByRole("article", { name: /Bạn/ });
    expect(region).toBeInTheDocument();
    expect(screen.getByText("Xin chào")).toBeInTheDocument();
  });

  it("nút sao chép tới được bằng bàn phím dù không rê chuột", async () => {
    // userEvent **tự cài** `navigator.clipboard` lúc `setup()`, nên mock phải
    // được cài **sau** `setup()` — cài trước sẽ bị nó ghi đè, và cài trước cả
    // `render` thì handler trong component vẫn chụp đúng object đó.
    const user = userEvent.setup();
    const writeText = vi.fn().mockResolvedValue(undefined);
    const descriptor = Object.getOwnPropertyDescriptor(globalThis.navigator, "clipboard");
    Object.defineProperty(globalThis.navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    try {
      renderRow(
        <MessageRow author="assistant" headingId="msg-2">
          Kết quả
        </MessageRow>,
      );
      const button = screen.getByRole("button", { name: "Sao chép tin nhắn" });
      // Bàn phím không có hover: nút vẫn phải focus được bằng Tab.
      button.focus();
      expect(button).toHaveFocus();
      await user.keyboard("{Enter}");
      expect(writeText).toHaveBeenCalledWith("Kết quả");
    } finally {
      if (descriptor) Object.defineProperty(globalThis.navigator, "clipboard", descriptor);
    }
  });

  it("vô hiệu hoá nút sao chép khi tin không có nội dung chữ", () => {
    renderRow(
      <MessageRow author="assistant">
        <span />
      </MessageRow>,
    );
    expect(screen.getByRole("button", { name: "Sao chép tin nhắn" })).toBeDisabled();
  });

  it("đổi nhãn Bean theo vai trò, không dùng chung nhãn chung chung", () => {
    const { rerender } = renderRow(
      <MessageRow author="assistant" headingId="msg-3">
        Chào
      </MessageRow>,
    );
    expect(screen.getByRole("article", { name: /Bean/ })).toBeInTheDocument();
    rerender(
      <I18nProvider>
        <MessageRow author="user" headingId="msg-4">
          Chào
        </MessageRow>
      </I18nProvider>,
    );
    expect(screen.getByRole("article", { name: /Bạn/ })).toBeInTheDocument();
  });
});

describe("MessageRow — an toàn", () => {
  it("nội dung do model sinh không bao giờ thành HTML thô", () => {
    const { container } = renderRow(
      <MessageRow author="assistant">{"<img src=x onerror=alert(1)>"}</MessageRow>,
    );
    // Nội dung đi qua React nên chữ hiện nguyên văn; không có phần tử `img` nào.
    expect(container.querySelector("img[src='x']")).toBeNull();
    expect(screen.getByText(/onerror/)).toBeInTheDocument();
  });

  it("vô hiệu hoá nút sao chép khi tin chỉ có khoảng trắng", () => {
    renderRow(<MessageRow author="assistant">{"  "}</MessageRow>);
    expect(screen.getByRole("button", { name: "Sao chép tin nhắn" })).toBeDisabled();
  });
});
