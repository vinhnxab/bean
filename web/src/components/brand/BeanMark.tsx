import { MARK_STROKE, MARK_VIEWBOX, type MarkVariant, markPaths } from "@/components/brand/markPaths";
import { cn } from "@/lib/utils";

/**
 * Mascot Bean dạng **vector** — dùng cho favicon và các vị trí quá nhỏ để ảnh
 * bitmap còn đọc được.
 *
 * # Ranh giới với `BeanAvatar` (ảnh thật)
 *
 * Hai component này **cố tình khác nhau**, không phải hai bản của cùng một thứ:
 *
 * - `BeanAvatar` (`/bean-avatar.png`) cho avatar 28–56px: ở đó bộ lông xoăn và
 *   bong bóng "?" đọc được, và đó mới là hình đại diện bạn muốn thấy.
 * - `BeanMark` (vector) cho favicon 16px: ở đó chi tiết bitmap vỡ thành vệt mực,
 *   còn đường nét vẽ thì sắc ở mọi tỉ lệ và chỉ 590 byte.
 *
 * `public/favicon.svg` được **sinh** từ `markPaths.ts` bằng `scripts/gen-favicon.ts`
 * nên favicon không bao giờ lệch với component này. `BeanAvatar` không sinh gì từ
 * vector — nó dùng ảnh gốc `web/brand/bean.png` của bạn.
 */
export function BeanMark({
  size,
  variant = "full",
  className,
  title,
}: {
  /** Cạnh vuông (px). Dùng số nguyên để nét không bị méo. */
  size: number;
  variant?: MarkVariant;
  className?: string;
  /** Nhãn trợ năng. Bỏ trống ⇒ ẩn khỏi cây trợ năng (dùng khi mascot chỉ trang trí). */
  title?: string;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox={MARK_VIEWBOX}
      fill="none"
      stroke="currentColor"
      strokeWidth={MARK_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={cn("shrink-0", className)}
      role={title ? "img" : "presentation"}
      aria-hidden={title ? undefined : true}
      aria-label={title}
      focusable="false"
    >
      {title ? <title>{title}</title> : null}
      {markPaths(variant).map((d) => (
        <path key={d.slice(0, 24)} d={d} />
      ))}
    </svg>
  );
}
