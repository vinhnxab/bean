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
 * Bề ngang cụm mắt = `280 - 198 = 82`px — hằng số **suy ra**, không đoán.
 *
 * Dùng làm mốc dưới cho cửa sổ cắt: khung phải rộng hơn bề ngang cụm mắt để
 * hai mắt không áp vào mép. Lấy `+14` px (7px mỗi bên) là biên vừa đủ để mắt
 * không "dính" khung khi phóng to.
 */
export const EYE_SPAN = EYES.right - EYES.left;
export const MIN_FACE_SIZE = EYE_SPAN + 14; // 96

/**
 * Cửa sổ cắt: **siết vào khuôn mặt**, không bong bóng, không vệt vòng viền.
 *
 * Chọn trên số đo, không bằng mắt:
 *
 * - **Mặt ở giữa**: hai mắt trải `198..280`, tâm mặt = **239**. Cửa sổ `189..289`
 *   có tâm 239 — khớp tuyệt đối, mắt đối xứng 9px mỗi bên.
 * - **Mép phải 289 < 315**: dừng trước bong bóng "?", biên an toàn 26px.
 * - Góc xa nhất `hypot(40, 91.5) ≈ 100` < 239 ⇒ trọn trong vòng viền.
 *
 * # Vì sao siết tới 100px
 *
 * Người dùng yêu cầu scale to hơn thay vì giữ tỉ lệ gốc. Đo bề rộng cụm mắt
 * (`82`px): ở bản cũ `136`px, mắt chỉ chiếm `82/136 ≈ 60%` khung — ở **28px
 * thật** avatar là một cục tối có hai chấm trắng. Soi 5 ứng viên ở đúng 28px:
 *
 * | crop | mắt chiếm | đọc được ở 28px |
 * |---|---|---|
 * | 136px | 60% | cục tối, mõm chưa ra |
 * | 124px | 66% | bắt đầu thấy mõm |
 * | 112px | 73% | có mặt |
 * | **100px** | **82%** | **rõ là khuôn mặt, có mõm** |
 * | 92px | 89% | sát mép, mõm bị cắt |
 *
 * Chọn **100px** — siết thêm 1.36× so với bản cũ.
 *
 * # Đính chính một lỗi trong chính file này
 *
 * Bản trước ghi *"dưới 150px là mất tai"* và test khoá cứng `size >= 150`. Con số
 * 150 đó **viết bằng mắt và sai**: đo lại, đầu kể cả tai trải `x 95..428` — rộng
 * **~300px**, nghĩa là crop 136px, kể cả 100px, **đều chưa bao giờ chứa trọn tai**.
 * Ở 28px tai vốn không phân giải được; thứ đọc được là **mắt + mõm**. Mốc dưới
 * nay lấy từ `MIN_FACE_SIZE` (bề ngang cụm mắt `+14`) thay vì hằng số bịa.
 *
 * Lỗi thứ hai ngay sau đó: tôi đặt tâm cửa sổ là 236 trong khi tâm mặt là 239,
 * khiến **mắt trái bị cắt mất 12px**. Test biên mắt mới bắt được; giờ tâm khớp
 * tuyệt đối.
 */
export const CROP = { x: 189, y: 158, size: 100 } as const;

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
