import { BADGE_BG_CSS } from "@/components/brand/crop";
import { cn } from "@/lib/utils";

/**
 * Avatar Bean — chú poodle đen trong huy hiệu tròn, **ảnh thật do bạn cung cấp**
 * (`web/brand/bean.png`), dùng ở mọi chỗ cần "hình đại diện của Bean".
 *
 * # Vì sao có cả `BeanAvatar` (ảnh) lẫn `BeanMark` (vector)
 *
 * Chúng phục vụ hai mục đích khác nhau, không thay thế được nhau:
 *
 * - **Avatar ≥ 24px** → dùng ảnh này. Ở 28–56px, bộ lông xoăn và bong bóng "?"
 *   đọc được và cho personality; vector cùng bộ nét sẽ chỉ là một hình sơ đồ.
 * - **Favicon 16px** → vẫn dùng `BeanMark` vector. Ở 16px, chi tiết của ảnh
 *   bitmap vỡ thành vệt mực, còn vector sắc nét ở mọi tỉ lệ và chỉ 590 byte.
 *
 * # Vì sao `/bean-avatar.png` chứ không nhúng ảnh gốc
 *
 * Ảnh gốc 500×500 nặng ~264 KB. File `public/bean-avatar.png` do
 * `scripts/gen-brand-assets.ts` sinh ra ở 128×128 (~31 KB) — đủ sắc cho avatar
 * 56px trên màn hình Retina 2x mà không kéo dài thời gian tải trang.
 */
export function BeanAvatar({
  size,
  className,
  title,
}: {
  /** Cạnh vuông (px). Ảnh gốc là huy hiệu tròn nên component bo tròn luôn. */
  size: number;
  className?: string;
  /** Nhãn trợ năng. Bỏ trắng ⇒ coi như trang trí, ẩn khỏi cây trợ năng. */
  title?: string;
}) {
  return (
    <img
      src="/bean-avatar.png"
      // Trình duyệt tự chọn bản 64px khi hiển thị nhỏ (avatar 28–32px trong chat),
      // nên phiên dài không phải giải mã hàng trăm bản 128×128.
      srcSet="/bean-avatar-sm.png 64w, /bean-avatar.png 128w"
      sizes={`${size}px`}
      // `width`/`height` cụ thể: chặn layout shift khi ảnh tải xong.
      width={size}
      height={size}
      alt={title ?? ""}
      aria-hidden={title ? undefined : true}
      // Nền be **không phải token của theme**: nó thuộc về chính bức ảnh. Nên ở
      // chế độ tối, nếu để trong suốt thì chú chó đen hoà vào nền tối và chỉ còn
      // một khối tối không đọc được. Lót tròn bằng màu be của huy hiệu — đúng màu
      // ở cả hai theme, nên hình tròn luôn "có mặt".
      //
      // `width`/`height` ở `style` là thứ **ràng buộc kích thước hiển thị**, còn
      // attribute cùng tên ở trên chỉ dành cho trình đọc màn hình và chống layout
      // shift. Không có CSS `height` thì `<img>` trong flex container (mặc định
      // `align-items: stretch`) bị kéo theo chiều cao nội dung bên cạnh ⇒ ở tin Bean
      // dài, avatar méo thành bầu dục (đo được: 128×4314). `shrink-0` không cứu
      // được vì nó chỉ chặn co theo **chiều ngang**.
      //
      // Không thêm `object-fit`: ảnh gốc vuông 128×128 và khung cũng vuông, nên
      // `cover`/`contain`/`fill` cho ra **cùng một kết quả** — đã so pixel thật
      // trên Chrome, cả bốn đều ra hash giống hệt. Không cần khoá ratio riêng:
      // đặt `width` bằng `height` đã đảm bảo khung vuông.
      style={{ width: size, height: size, backgroundColor: BADGE_BG_CSS }}
      className={cn("shrink-0 rounded-full", className)}
      draggable={false}
    />
  );
}

/**
 * Mascot Bean (poodle) — component dùng chung cho **mọi** vị trí hiển thị.
 *
 * 5 vị trí: favicon (`public/favicon.svg`, sinh từ `markPaths`), logo HUB (44px),
 * avatar Manager trong chat (28px), skeleton loading (24px), trạng thái rỗng (56px).
 *
 * Dùng `currentColor` nên mascot tự nhận màu của vùng chứa — đó là lý do "chó poodle"
 * là **thứ ấm duy nhất** trên màn hình mà vẫn không phải một khối màu trang trí.
 */
