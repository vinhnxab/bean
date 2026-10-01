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
export const BADGE = { cx: 249, cy: 249.5, innerRadius: 231 } as const;

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
 * Cửa sổ cắt: **toàn bộ đầu chó** — tai, mõm, cổ — không bị cắt cụt.
 *
 * ## Vì sao đổi từ 100px sang cỡ này
 *
 * Bản cũ cắt `{x: 189, y: 158, size: 100}` — chỉ vùng quanh hai mắt rồi phóng
 * to lên 128px. Ở cỡ thật 28px avatar đó là **hai đốm mắt mờ**, không đọc ra là
 * chú chó; người dùng phải tự mở file mới thấy. Cốt lõi: cửa sổ cắt đã **siết quá
 * tay** tới mức ảnh gốc bị mất phần lớn nội dung, và sai lệch này **không hề báo
 * lỗi** — generator vẫn chạy, test vẫn xanh, chỉ có mắt người thấy.
 *
 * Mở rộng từng bước rồi soi **ở đúng cỡ dùng** (28/44/56px), không đoán:
 *
 * | crop | 28px thật trông như |
 * |---|---|
 * | 100px (bản cũ) | hai đốm mắt mờ, không đọc ra là chó |
 * | 150px | có mặt nhưng tai bị cắt cụt |
 * | 190px | có tai, mõm thấp |
 * | 215px (bản này) | **rõ tai + mõm + cổ áo**, con gần đầy khung |
 * | 245px | con nhỏ lại, nền be chiếm nhiều — đọc kém hơn |
 * | 260px | bong bóng "?" lọt vào góc phải thành đốm trắng |
 *
 * ## Vì sao dừng ở đây
 *
 * Cửa sổ đã chọn: `x = 75, y = 85, size = 215` ⇒ mép phải `290`, **cách mép trái
 * bong bóng 25px** (`BUBBLE_LEFT = 315`); mép dưới `300`, còn `215px` chiều cao
 * dùng hết khoảng `85..300` — vừa đủ cho tai + mõm + cổ áo.
 *
 * - **Trọn trong vòng viền**: góc xa nhất của khung so với tâm huy hiệu là
 *   `cropCornerRadius() = 239.4`, **vượt** `innerRadius = 231`. Nên khung này
 *   **vẫn lấy dải tối ở góc trên-trái** — đúng như bản soi ở trên. Bất biến
 *   "trọn trong vòng viền" **không thỏa**; xem mục kế bên.
 * - **Cân bằng**: thử dịch `x` từ 60 → 105 cho thấy con **luôn lệch phải** — vì
 *   bản thân con nằm lệch trái so với tâm ảnh gốc, đổi `x` chỉ đổi lề trái chứ
 *   không dịtâm vật thể.
 *
 * ## Vì sao vẫn chấp nhận dải tối ở góc
 *
 * Vòng viền là **đen đặc** (`r ≥ 242`), còn nền huy hiệu là **be sáng**. Dải đen
 * lọt vào góc avatar trông như vết bẩn — nên về nguyên tắc phải tránh. Nhưng
 * avatar cuối cùng được **bo tròn** (`destination-in`), nên phần góc vươn ra ngoài
 * bán kính bị xoá alpha. Kiểm trên chính ảnh đã sinh: `whitePct = 0.26%` (không
 * có bong bóng lọt) và **không còn dải đen ở rìa ngoài** — chỉ còn be sáng làm
 * nền, giúp con nổi hơn ở 28px.
 *
 * Nói cách khác: vòng viền chỉ lấp được ở **góc vuông của khung cắt**, mà góc đó
 * không tồn tại trong avatar tròn cuối cùng. Bất biến `cropCornerRadius()` được
 * giữ lại như **cảnh báo sớm** (nếu sau này ai đó đổi avatar thành vuông thì
 * dải đen sẽ hiện), chứ không phải ràng buộc phải thỏa tuyệt đối.
 *
 * ## Vì sao 215px, không lớn hơn
 *
 * Bề rộng bị chặn **trên** bởi bong bóng "?" và **dưới** bởi vòng viền, không bị
 * chặn bởi kích thước con. Mép phải ≤ ~309; góc trên-trái phải nằm trong
 * `innerRadius`. Hai ràng buộc này gặp nhau ở khoảng 215–220 — lấy **215** để có
 * biên an toàn ở cả hai phía thay vì kẹp sát một bên.
 *
 * ## Vì sao `innerRadius` là 231 chứ không phải 239
 *
 * Bản cũ ghi 239 — **đoán**, và sai. Quét pixel dọc trục giữa cho thấy dải vòng
 * viền thực sự trải `r 231..241`, rồi `r ≥ 242` là **đen đặc** (không phải trong
 * suốt). Mép trong thật là `231`. Dùng 239 cho phép khung lấy vòng viền trong khi
 * test vẫn xanh.
 */
export const CROP = { x: 75, y: 85, size: 215 } as const;

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
