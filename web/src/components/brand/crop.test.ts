import { describe, expect, it } from "vitest";
import { BADGE, BUBBLE_LEFT, bubbleMargin, CROP, cropCornerRadius, cropRightEdge, EYES } from "./crop";

describe("cửa sổ cắt avatar", () => {
  // Đây chính là lỗi đã xảy ra: crop cũ {x:104,y:54,size:236} có mép phải 340,
  // lọt vệt trắng bong bóng "?" vào avatar 28px. Generator vẫn chạy, không test
  // nào đỏ — chỉ khi soi ảnh ở kích thước thật mới thấy.
  it("dừng trước mép trái của bong bóng suy nghĩ", () => {
    expect(cropRightEdge()).toBeLessThanOrEqual(BUBBLE_LEFT);
  });

  // Chỉ "dừng trước" thì chưa đủ: sát mép nghĩa là một lần thu nhỏ ảnh là lọt.
  it("giữ biên an toàn với bong bóng, không sát mép", () => {
    expect(bubbleMargin()).toBeGreaterThanOrEqual(6);
  });

  it("nằm trọn bên trong vòng viền của huy hiệu", () => {
    expect(cropCornerRadius()).toBeLessThan(BADGE.innerRadius);
  });

  it("cửa sổ là hình vuông nằm gọn trong ảnh gốc", () => {
    expect(CROP.x).toBeGreaterThanOrEqual(0);
    expect(CROP.y).toBeGreaterThanOrEqual(0);
    expect(CROP.x + CROP.size).toBeLessThanOrEqual(500);
    expect(CROP.y + CROP.size).toBeLessThanOrEqual(500);
  });

  // Lỗi thứ hai đã xảy ra: crop cũ {x:100,y:90,size:200} đẩy mặt sát mép phải
  // nên **cắt mất mõm**, và để lọt nêm nền be ở góc trên-trái — ở 28px nêm đó đọc
  // như cái mũ. Hai mắt (đo được ở 198..280) phải nằm trọn trong khung.
  it("giữ trọn cả hai mắt, không cắt mõm", () => {
    expect(CROP.x).toBeLessThanOrEqual(EYES.left);
    expect(cropRightEdge()).toBeGreaterThanOrEqual(EYES.right);
  });

  // Ở 28px, mặt phải chiếm phần lớn khung; crop quá rộng thì tai và bông bớt
  // nổi, chó thành một vệt nâu. Dưới 150px là mất tai — dấu hiệu nhận dạng poodle.
  it("đủ rộng để giữ tai nhưng không thừa", () => {
    expect(CROP.size).toBeGreaterThanOrEqual(150);
    expect(CROP.size).toBeLessThanOrEqual(236);
  });

  // Bảo vệ bước đo mà các hằng số trên dựa vào: nếu ai đó thay `bean.png` bằng
  // ảnh khác, các mốc này phải được đo lại chứ không âm thầm sai.
  it("các mốc đo nằm trong ảnh gốc", () => {
    expect(BUBBLE_LEFT).toBeGreaterThan(EYES.right);
    expect(EYES.left).toBeGreaterThan(0);
    expect(EYES.right).toBeLessThan(500);
    expect(BADGE.innerRadius).toBeGreaterThan(0);
  });
});
