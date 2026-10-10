import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { QueuePill, StreamingCaret, ThinkingIndicator } from "@/components/chat/RunStatus";
import { I18nProvider } from "@/i18n";

function renderStatus(ui: React.ReactNode) {
  return render(<I18nProvider>{ui}</I18nProvider>);
}

describe('RunStatus — trạng thái "đang suy nghĩ"', () => {
  it("ThinkingIndicator: role=status + nhãn, chi tiết trang trí ẩn với trình đọc màn hình", () => {
    renderStatus(<ThinkingIndicator />);
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Đang suy nghĩ…");
    // Ba chấm nhảy là trang trí: không được đọc thành chữ.
    expect(status.querySelector('[aria-hidden="true"]')).not.toBeNull();
    expect(status.querySelector("img")?.getAttribute("alt")).toBe("");
  });

  it("QueuePill hiển thị vị trí trong hàng đợi", () => {
    renderStatus(<QueuePill position={2} />);
    expect(screen.getByRole("status")).toHaveTextContent("Đang chờ · #2");
  });

  it("QueuePill không rõ vị trí thì chỉ hiện nhãn chung chung", () => {
    renderStatus(<QueuePill position={null} />);
    expect(screen.getByRole("status")).toHaveTextContent("Đang chờ");
  });

  it("StreamingCaret là chi tiết trang trí, ẩn khỏi cây trợ năng", () => {
    const { container } = renderStatus(<StreamingCaret />);
    const caret = container.querySelector('[aria-hidden="true"]');
    expect(caret).not.toBeNull();
    // Không có text node trần nào ngoài label — caret không được thêm giọng đọc.
    expect(caret?.textContent).toBe("");
  });
});
