import { describe, expect, it } from "vitest";
import { BADGE_BG, BADGE_BG_CSS, BG_PROBE_POINTS, BG_TOLERANCE, SOURCE_EDGE } from "./badge";

describe("hằng số huy hiệu Bean", () => {
  // Ảnh gốc là hình vuông; nếu ai đó thay `bean.png` bằng ảnh cỡ khác thì
  // `SOURCE_EDGE` phải được đo lại — generator cũng tự kiểm và báo lỗi, test này
  // chỉ chốt con số đã biết.
  it("ảnh gốc là hình vuông 500×500", () => {
    expect(SOURCE_EDGE).toBe(500);
  });

  // Điểm dò màu phải nằm trong ảnh, nếu không `getImageData` ra pixel rỗng và
  // trung vị màu nền sai — generator lót icon bằng màu của chính chú chó.
  it("mọi điểm dò màu nằm trong ảnh gốc", () => {
    expect(BG_PROBE_POINTS.length).toBeGreaterThanOrEqual(2);
    for (const [x, y] of BG_PROBE_POINTS) {
      expect(x).toBeGreaterThanOrEqual(0);
      expect(y).toBeGreaterThanOrEqual(0);
      expect(x).toBeLessThan(SOURCE_EDGE);
      expect(y).toBeLessThan(SOURCE_EDGE);
    }
  });

  it("màu nền là kênh RGB hợp lệ", () => {
    for (const channel of [BADGE_BG.r, BADGE_BG.g, BADGE_BG.b]) {
      expect(channel).toBeGreaterThanOrEqual(0);
      expect(channel).toBeLessThanOrEqual(255);
    }
    expect(BG_TOLERANCE).toBeGreaterThan(0);
  });

  it("chuỗi CSS khớp hằng số RGB", () => {
    expect(BADGE_BG_CSS).toBe(`rgb(${BADGE_BG.r}, ${BADGE_BG.g}, ${BADGE_BG.b})`);
  });
});
