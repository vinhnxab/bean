import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { formatElapsed, QueuePill, ThinkingIndicator } from "@/components/chat/RunStatus";
import { I18nProvider } from "@/i18n";

function renderStatus(ui: React.ReactNode) {
  return render(<I18nProvider>{ui}</I18nProvider>);
}

describe('RunStatus — trạng thái "đang suy nghĩ"', () => {
  it("ThinkingIndicator: role=status + nhãn + đồng hồ đã trôi, chi tiết trang trí ẩn với trình đọc màn hình", () => {
    renderStatus(<ThinkingIndicator startedAt={Date.now() - 5000} />);
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Đang suy nghĩ…");
    expect(status).toHaveTextContent("5s");
    // Ba chấm nhảy là trang trí: không được đọc thành chữ.
    expect(status.querySelector('[aria-hidden="true"]')).not.toBeNull();
    expect(status.querySelector("img")?.getAttribute("alt")).toBe("");
  });

  it("ThinkingIndicator chưa đầy một giây thì chưa hiện đồng hồ", () => {
    renderStatus(<ThinkingIndicator startedAt={Date.now()} />);
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Đang suy nghĩ…");
    expect(status.textContent).not.toContain("0s");
  });

  it("ThinkingIndicator không có mốc giờ (run lạ) thì chỉ hiện nhãn", () => {
    renderStatus(<ThinkingIndicator startedAt={null} />);
    expect(screen.getByRole("status").textContent).toBe("Đang suy nghĩ…");
  });

  it("ThinkingIndicator khi stopping đổi nhãn thành Đang dừng…", () => {
    renderStatus(<ThinkingIndicator startedAt={Date.now() - 3000} stopping />);
    expect(screen.getByRole("status")).toHaveTextContent("Đang dừng…");
  });

  it("formatElapsed: dưới một phút là giây, trên một phút là phút:giây", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(5)).toBe("5s");
    expect(formatElapsed(59)).toBe("59s");
    expect(formatElapsed(60)).toBe("1:00");
    expect(formatElapsed(65)).toBe("1:05");
  });

  it("QueuePill hiển thị vị trí trong hàng đợi", () => {
    renderStatus(<QueuePill position={2} />);
    expect(screen.getByRole("status")).toHaveTextContent("Đang chờ · #2");
  });

  it("QueuePill không rõ vị trí thì chỉ hiện nhãn chung chung", () => {
    renderStatus(<QueuePill position={null} />);
    expect(screen.getByRole("status")).toHaveTextContent("Đang chờ");
  });
});
