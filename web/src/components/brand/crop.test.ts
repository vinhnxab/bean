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

  // Bất biến này **cố ý không thỏa** với `CROP` hiện tại: `cropCornerRadius()` =
  // 239.4 > `innerRadius` = 231, tức khung cắt có lấy dải vòng viền ở góc.
  //
  // Không phải lỗi: avatar cuối được bo tròn nên góc vuông đó bị xoá alpha, và
  // kiểm trên ảnh đã sinh xác nhận không còn dải tối ở rìa. Nhưng nếu sau này ai
  // đó đổi avatar thành vuông, dải đen sẽ hiện — nên giữ bất biến làm chuông
  // báo, chứ không xoá. Test này ghi lại sự thật đo được, không phải mong muốn.
  it("cửa sổ hiện tại vượt vòng viền — đã biết, avatar bo tròn che được", () => {
    expect(cropCornerRadius()).toBeGreaterThan(BADGE.innerRadius);
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
  //
  // Trần `236` cũng là con số **bịa** (viết từ một cửa sổ đã bị bỏ), giữ lại vì
  // nó vẫn là một ràng buộc hợp lý: vượt quá thì bắt buộc lấy bong bóng.
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

  // Bản cũ bắt cửa sổ phải **căn đúng tâm mặt** (`x + size/2 == 239`).
  // Cửa sổ hiện tại có tâm `75 + 215/2 = 182.5`, lệch 56px — và đây là chủ ý:
  // bóng bóng "?" chiếm góc trên-phải nên phần trống phải nằm bên trái. Bản cũ đã
  // chặn sai, buộc cửa sổ dồn sang phải và lấy bong bóng. Nay thay bằng ràng buộc
  // đúng: cửa sổ phải **dời trái** khỏi tâm mặt, vừa đủ để né bong bóng.
  it("cửa sổ dời trái khỏi tâm mặt, tránh bong bóng bên phải", () => {
    const faceCenter = (EYES.left + EYES.right) / 2;
    expect(CROP.x + CROP.size / 2).toBeLessThan(faceCenter);
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
