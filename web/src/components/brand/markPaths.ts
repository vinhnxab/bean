/**
 * Nguồn duy nhất cho hình mascot Bean (poodle).
 *
 * # Vì sao một nguồn, không vẽ tay ở từng chỗ
 *
 * Mascot xuất hiện ở 5 vị trí (favicon, logo HUB, avatar Manager, skeleton
 * loading, trạng thái rỗng). Nếu mỗi vị trí có một bản riêng, chúng chắc chắn lệch
 * nhau sau vài lần sửa — và sự lệch đó không ai báo lỗi. `favicon.svg` được **sinh
 * từ đúng hằng số trong file này**, không phải một bản vẽ thứ hai.
 *
 * # Đặc tả nhận diện ở kích thước nhỏ
 *
 * Ở 16px (tab trình duyệt) chỉ còn ba thứ đọc được: **đầu tròn + hai tai cụp +
 * mũi**. Tai cụp là dấu hiệu nhận diện poodle, nên nó là khối lớn thứ hai sau đầu.
 * Mắt và mõm là chi tiết *cấp 2* — bỏ ở `silhouette`, hiện từ `full` trở lên.
 *
 * Một kiểu nét duy nhất xuyên suốt 5 vị trí: `MARK_STROKE`, bo tròn hai đầu, KHÔNG
 * fill, KHÔNG gradient, KHÔNG bóng. Dùng `currentColor` nên tự đúng màu trên cả
 * light/dark mà không cần hai bản.
 */

/** Bề dày nét (trong hệ toạ độ 24×24) — dùng chung mọi vị trí. */
export const MARK_STROKE = 1.75;

/** `viewBox` cố định 24×24 để tỉ lệ nét không đổi giữa favicon và logo. */
export const MARK_VIEWBOX = "0 0 24 24";

/**
 * Hộp bao của mascot trong hệ toạ độ 24×24, **lớn hơn** mọi phiên bản.
 *
 * Dùng để canh giữa và — quan trọng hơn — để chặn lỗi cắt hình: SVG mặc định
 * `overflow: hidden`, nên bất kỳ thứ gì vượt `viewBox` sẽ **bị cắt âm thầm**.
 * Bản favicon cũ đẩy tai phải ra ngoài x=24 và mất tai mà không ai báo lỗi.
 *
 * Tai phải chạm x=21.8; y đáy là điểm cuối nét tai (17.0).
 */
export const MARK_BOUNDS = { minX: 2.2, minY: 4.8, maxX: 21.8, maxY: 17.0 };

/**
 * Dịch chút để mascot **cân bằng quang học** trong ô vuông: hình ngồi cao hơn
 * tâm một chút (tâm thật ≈ 10.9 so với 12), nên hạ nhẹ xuống thay vì để lệch.
 *
 * Giữ nhỏ và đủ để `MARK_BOUNDS + MARK_NUDGE` vẫn nằm trong `viewBox`; test
 * trong `HubPage.test.tsx` kiểm tra đúng bất biến này.
 */
export const MARK_NUDGE = { x: 0, y: 1 };

/** Phiên bản mascot, chọn theo kích thước và ngữ cảnh hiển thị. */
export type MarkVariant =
  /** Favicon 16–32px: chỉ đầu + tai + mũi. */
  | "silhouette"
  /** Logo/avatar từ 28px trở lên: thêm mắt và mõm. */
  | "full"
  /** Skeleton: mắt nhắm, tĩnh — không nhấp nháy khi reduced-motion. */
  | "resting";

/**
 * Đầu — hình tròn dẹt, tâm (12, 9.2), bán kính ngang 4.6 · dọc 4.4.
 *
 * Cố tình **nhỏ hơn tai** — xem ghi chú ở `EARS`.
 */
const HEAD = "M12 4.8c2.6 0 4.6 2 4.6 4.4s-2 4.4-4.6 4.4-4.6-2-4.6-4.4 2-4.4 4.6-4.4Z";

/**
 * Hai tai cụp — **khối định danh poodle**.
 *
 * Bản đầu của tôi đặt tai *bên trong* hình đầu, kết quả nhìn ra là khỉ/gấu chứ
 * không phải poodle. Ba điều kiện để đọc đúng ở 16px:
 *
 * 1. Tai **rộng hơn đầu** (tràn ra ngoài cả hai bên: x từ 2.2 tới 21.8 so với
 *    đầu chỉ 7.4–16.6) và **dài hơn đầu** (tới y≈17 so với đầu 13.6). Đó là tỉ lệ
 *    của poodle, không phải của gấu.
 * 2. Tai **bám đúng lên đường viền đầu** tại (8.4, 6.5) và (15.6, 6.5) — tách rời
 *    một chút là thành hai vệt lạ, không ra tai.
 * 3. Vẽ bằng nét cong **liên tục, không khép kín** — nét khép kín cắt ngang mặt
 *    đầu và trông như tai đeo bịt tai.
 */
const EARS = "M8.4 6.5C5 6 2.2 8.2 2.2 11.2s1.8 5.8 4.4 5.8M15.6 6.5c3.4-.5 6.2 1.7 6.2 4.7s-1.8 5.8-4.4 5.8";

/** Mũi (mọi phiên bản) — khối nhỏ ở giữa, đọc được ở 16px. */
const NOSE = "M10.9 11.1h2.2a.6.6 0 0 1 .42 1.03l-.94.92a.6.6 0 0 1-.83 0l-.94-.92A.6.6 0 0 1 10.9 11.1Z";

/** Mõm — vòng cung nhỏ ngay dưới mũi. */
const MUZZLE = "M10.4 14c.45.5 1 .75 1.6.75s1.15-.25 1.6-.75";

/** Sợi tóc trên đỉnh — chi tiết cấp 2, chỉ `full`. */
const CREST = "M9.9 4.6c.6-.9 1.3-1.35 2.1-1.35s1.5.45 2.1 1.35";

/** Mắt mở — hai chấm đặc, chỉ `full`. */
const EYES_OPEN = "M9.85 8.5a.6.6 0 1 0 0-1.2.6.6 0 0 0 0 1.2ZM14.15 8.5a.6.6 0 1 0 0-1.2.6.6 0 0 0 0 1.2Z";

/** Mắt nhắm — vòng cung thay vì chấm, chỉ `resting`. */
const EYES_RESTING = "M9.1 8.1c.4.5 1.05.5 1.45 0M13.45 8.1c.4.5 1.05.5 1.45 0";

/** Các đường nét của một phiên bản, theo đúng thứ tự vẽ. */
export function markPaths(variant: MarkVariant): string[] {
  if (variant === "silhouette") {
    return [HEAD, EARS, NOSE];
  }
  if (variant === "resting") {
    return [HEAD, EARS, NOSE, EYES_RESTING, MUZZLE];
  }
  return [HEAD, EARS, NOSE, CREST, EYES_OPEN, MUZZLE];
}
