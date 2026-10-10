import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Enter, prefersReducedMotion } from "@/lib/anim";

/**
 * `Enter` là lớp hoạt ảnh bọc quanh nội dung chat — nên test tập trung vào bất
 * biến quan trọng nhất: **nội dung không bao giờ phụ thuộc vào animation**.
 * (jsdom có `matchMedia` nhờ polyfill trong `src/test/setup.ts`, nên đường GSAP
 * thật cũng chạy trong các test này — lỗi API sẽ lộ ra ngay.)
 */
describe("Enter", () => {
  it("luôn render nội dung con — hoạt ảnh chỉ là trang trí", () => {
    render(
      <Enter kind="rise">
        <p>Không được mất</p>
      </Enter>,
    );
    expect(screen.getByText("Không được mất")).toBeInTheDocument();
  });

  it("when=false (nền cũ khi mở phiên) không để lại style inline ẩn phần tử", () => {
    const { container } = render(
      <Enter when={false} kind="rise">
        <p>Lịch sử tải sẵn</p>
      </Enter>,
    );
    const wrapper = container.firstElementChild;
    expect(wrapper).not.toBeNull();
    // `gsap.from` ghi `opacity` ngay khi chạy; `when=false` không tạo tween nào
    // nên phần tử phải visible ngay từ frame đầu — mở phiên cũ không được nháy.
    expect((wrapper as HTMLElement).style.opacity).toBe("");
    expect((wrapper as HTMLElement).style.transform).toBe("");
  });

  it("giảm chuyển động → hiển thị nguyên trạng thái, không tạo tween", () => {
    const original = window.matchMedia;
    // Lưu ý chuỗi truy vấn gốc "(prefers-reduced-motion: …)" CHỨA cả hai từ
    // "reduce" và "no-preference" — chỉ match đúng khi có "reduce" mà KHÔNG có
    // "no-preference".
    window.matchMedia = ((query: string) =>
      ({
        matches: query.includes("reduce") && !query.includes("no-preference"),
        media: query,
        onchange: null,
        addEventListener: () => {},
        removeEventListener: () => {},
        addListener: () => {},
        removeListener: () => {},
        dispatchEvent: () => false,
      }) as MediaQueryList) as typeof window.matchMedia;
    try {
      expect(prefersReducedMotion()).toBe(true);
      const { container } = render(
        <Enter kind="pop">
          <p>Thấy ngay</p>
        </Enter>,
      );
      expect(screen.getByText("Thấy ngay")).toBeInTheDocument();
      expect((container.firstElementChild as HTMLElement).style.opacity).toBe("");
    } finally {
      window.matchMedia = original;
    }
  });
});
