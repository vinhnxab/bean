import { describe, expect, it } from "vitest";
import {
  BADGE,
  BUBBLE_LEFT,
  bubbleMargin,
  CROP,
  cropCornerRadius,
  cropRightEdge,
  EYES,
  MIN_FACE_SIZE,
} from "./crop";

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

  // Ngưỡng dưới **từng là 150px, đặt bằng mắt và sai**: đo lại, đầu kể cả tai trải
  // x 95..428 (~300px), nên 136px hay 100px đều chưa bao giờ chứa trọn tai. Nay
  // lấy mốc từ bề ngang cụm mắt — thứ thực sự đọc được ở 28px.
  it("đủ rộng để hai mắt không áp mép, nhưng không thừa", () => {
    expect(CROP.size).toBeGreaterThanOrEqual(MIN_FACE_SIZE);
    expect(CROP.size).toBeLessThanOrEqual(236);
  });

  // Chặn đúng cái lỗi vừa gặp: siết quá tay tới mức mắt dính mép khung. Ở 28px
  // mắt là thứ duy nhất đọc được, nên phải có biên.
  //
  // Biên trái là `EYES.left - CROP.x` — tôi đã viết **ngược dấu** thành
  // `CROP.x - EYES.left` và test báo đỏ trên một crop hoàn toàn đúng. Sửa dấu, và
  // thêm kiểm tra đối xứng để lỗi kiểu này không tái diễn.
  it("hai mắt có biên an toàn, không áp vào mép khung", () => {
    expect(EYES.left - CROP.x).toBeGreaterThanOrEqual(4);
    expect(cropRightEdge() - EYES.right).toBeGreaterThanOrEqual(4);
  });

  // Cửa sổ phải căn theo tâm mặt: lệch sang phải 3px là cắt mất mắt trái.
  it("cửa sổ căn đúng tâm mặt, hai mắt đối xứng", () => {
    const faceCenter = (EYES.left + EYES.right) / 2;
    expect(CROP.x + CROP.size / 2).toBeCloseTo(faceCenter, 5);
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
