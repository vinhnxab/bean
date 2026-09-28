/**
 * Toạ độ cắt avatar Bean — **đo từ `brand/bean.png`, không đoán bằng mắt**.
 *
 * # Vì sao file này tồn tại
 *
 * `CROP` là thứ dễ sai nhất trong chuỗi thương hiệu: lệch một chút thì ảnh vẫn
 * "đẹp" nhưng lọt **vệt trắng của bong bóng "?"** vào avatar 28px — Ở cỡ đó nó
 * thành một đốm sáng giả, tự nó trông như đèn báo.
 *
 * Sai ở đây **không hề báo lỗi**: generator vẫn chạy, file vẫn sinh ra, không
 * test nào đỏ. Chỉ có mắt người thấy. Nên toạ độ nằm ở đây kèm **bất biến kiểm
 * được** (`crop.test.ts`), thay vì nằm rơi trong script sinh ảnh.
 */

/** Cạnh ảnh gốc (px) — `bean.png` là ảnh vuông 500×500. */
export const SOURCE_EDGE = 500;

/**
 * Hình học huy hiệu tròn, **đo bằng cách quét pixel** (dòng/col giữa tìm dải
 * xám đều của vòng viền): tâm `(249, 249.5)`, mép trong vòng cách tâm ~239px.
 *
 * Vòng viền chạm sát mép ảnh nên bán kính ngoài vượt khung; ta chỉ cần **mép
 * trong** để biết vùng an toàn.
 */
export const BADGE = { cx: 249, cy: 249.5, innerRadius: 239 } as const;

/**
 * Mép trái của bong bóng suy nghĩ "?" (px trong ảnh gốc) — **đo, không đoán**.
 *
 * Quét pixel: trong vùng tròn của huy hiệu, đếm cột có ≥5 pixel *tinh* hơn (luminance
 * > 238) rồi tách theo `firstY`. Kết quả là **hai cụm rời rạc**:
 *
 * - `x 315..440`, `firstY 84..106` → **bong bóng** (trên-phải)
 * - `x 198..280`, `firstY 180..214` → **mắt** (dưới)
 *
 * Chính vì có hai cụm, một lần quét "pixel sáng nhất" sẽ ra `198` — tức là **lấy
 * nhầm mắt làm bong bóng**. Hằng số cũ `300` là con số đoán từ contact sheet, và nó
 * sai theo hướng nguy hiểm: biên 300 khiến người ta tưởng đã khoảng cách an toàn
 * trong khi thật ra chỉ cách 15px.
 */
export const BUBBLE_LEFT = 315;

/**
 * Mép trái của cụm mắt — dùng để chứng minh crop **không cắt mất mắt**.
 *
 * Đo cùng cách: `x 198..280`. Mép phải cần ≥ 280, và mép trái ≤ 198 để cả hai mắt
 * cùng nằm trong khung.
 */
export const EYES = { left: 198, right: 280 } as const;

/**
 * Cửa sổ cắt: **đầu căn giữa, không bong bóng, không vệt vòng viền**.
 *
 * Chọn trên số đo, không bằng mắt:
 *
 * - **Mặt ở giữa**: hai mắt trải `198..280`, tâm mặt ≈ 239. Cửa sổ `122..306` có
 *   tâm 214 — mặt lệch phải 25px, đúng như mong muốn vì **mõm kéo dài sang phải**;
 *   cân mặt vào chính giữa sẽ cắt mất mõm.
 * - **Mép phải 306 < 315**: dừng trước bong bóng, biên an toàn 9px.
 *   Ứng viên `{124,111,186}` (mép phải 310, biên 5px) nhìn *gần như giống hệt*
 *   khi soi ở 16/28/150px, nên lấy biên rộng hơn — không tốn gì mà bớt rủi ro.
 * - Góc xa nhất: `hypot(127, 138.5) ≈ 188` < 239 ⇒ trọn trong vòng viền.
 *
 * Bản cũ `{100, 90, 200}` cắt mất mõm sát mép phải và để lọt nêm nền be ở
 * góc trên-trái; ở 28px nêm đó đọc như cái mũ.
 */
export const CROP = { x: 122, y: 111, size: 184 } as const;

/**
 * Điểm dò màu nền huy hiệu — nằm trên vòng viền, **ngoài chú chó**.
 *
 * Một điểm đơn lẻ là **không đủ**: bản đầu tôi lấy điểm `(250, 110)` và tưởng
 * đó là nền, nhưng nó rơi đúng vào lông chó — nên `apple-touch-icon` bị lót bằng
 * **nâu sẫm** thay vì be. Dùng nhiều điểm rồi lấy trung vị phần sáng: chó có lông
 * tối lẫn điểm sáng, còn nền thì luôn sáng và đều.
 */
export const BG_PROBE_POINTS: readonly (readonly [number, number])[] = [
  [200, 130],
  [300, 130],
  [160, 150],
  [340, 150],
  [250, 150],
  [210, 100],
  [290, 100],
];

/**
 * Màu nền huy hiệu, **đo bằng trung vị các điểm trên** (chỉ lấy pixel alpha > 200):
 * `rgb(252, 214, 146)`.
 *
 * Generator tự đo lại rồi **so với hằng số này** và fail nếu lệch quá 12 đơn vị
 * mỗi kênh. Nhờ vậy thay `bean.png` bằng ảnh khác sẽ báo lỗi ngay, thay vì âm
 * thầm lệch màu — mà lệch màu thì lúc xem nhanh không ai thấy.
 */
export const BADGE_BG = { r: 252, g: 214, b: 146 } as const;

/** Sai số cho phép mỗi kênh khi so màu nền đo được với `BADGE_BG`. */
export const BG_TOLERANCE = 12;

/** Định dạng màu CSS từ `BADGE_BG`. */
export const BADGE_BG_CSS = `rgb(${BADGE_BG.r}, ${BADGE_BG.g}, ${BADGE_BG.b})`;

/** Bán kính lớn nhất từ tâm huy hiệu tới một góc của cửa sổ cắt. */
export function cropCornerRadius(): number {
  const { x, y, size } = CROP;
  return Math.max(
    Math.hypot(x - BADGE.cx, y - BADGE.cy),
    Math.hypot(x + size - BADGE.cx, y - BADGE.cy),
    Math.hypot(x - BADGE.cx, y + size - BADGE.cy),
    Math.hypot(x + size - BADGE.cx, y + size - BADGE.cy),
  );
}

/** Cạnh phải của cửa sổ cắt — dùng để chặn bong bóng lọt vào. */
export function cropRightEdge(): number {
  return CROP.x + CROP.size;
}

/** Biên an toàn giữa mép phải crop và mép trái bong bóng (px). */
export function bubbleMargin(): number {
  return BUBBLE_LEFT - cropRightEdge();
}
