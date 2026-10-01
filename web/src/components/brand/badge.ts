/**
 * Hằng số của **huy hiệu Bean** — nguồn duy nhất cho ảnh mascot/icon.
 *
 * # Vì sao file này tồn tại
 *
 * `brand/bean.png` là một ảnh vuông 500×500 vẽ chú poodle trong huy hiệu tròn
 * (nền be, có vòng viền, kèm bong bóng "?"). Avatar và icon đều **sinh ra** từ
 * ảnh này bằng `scripts/gen-brand-assets.ts`, và mọi kích thước chỉ **thu nhỏ
 * trọn ảnh** (scale) — **không cắt** — nên nội dung luôn còn nguyên ở mọi cỡ.
 *
 * Sai ở đây **không hề báo lỗi**: generator vẫn chạy, file vẫn sinh ra, không test
 * nào đỏ. Chỉ có mắt người thấy. Nên các hằng số nằm ở đây kèm **bất biến kiểm
 * được** (`badge.test.ts`), thay vì nằm rơi trong script sinh ảnh.
 */

/** Cạnh ảnh gốc (px) — `bean.png` là ảnh vuông 500×500. */
export const SOURCE_EDGE = 500;

/**
 * Điểm dò màu nền huy hiệu — nằm trên vòng viền, **ngoài chú chó**.
 *
 * Một điểm đơn lẻ là **không đủ**: bản đầu lấy điểm `(250, 110)` và tưởng đó là
 * nền, nhưng nó rơi đúng vào lông chó — nên `apple-touch-icon` bị lót bằng **nâu
 * sẫm** thay vì be. Dùng nhiều điểm rồi lấy trung vị phần sáng: chó có lông tối
 * lẫn điểm sáng, còn nền thì luôn sáng và đều.
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
