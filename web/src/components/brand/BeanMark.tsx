import { MARK_STROKE, MARK_VIEWBOX, type MarkVariant, markPaths } from "@/components/brand/markPaths";
import { cn } from "@/lib/utils";

/**
 * Mascot Bean dạng **vector** — dùng cho các vị trí nhỏ, cần màu theo
 * `currentColor` (trạng thái rỗng, trang trí).
 *
 * # Ranh giới với `BeanAvatar` (ảnh thật)
 *
 * Hai component này **cố tình khác nhau**, không phải hai bản của cùng một thứ:
 *
 * - `BeanAvatar` (`/bean-avatar.png`) cho avatar 28–56px **và favicon**: ở đó bộ
 *   lông xoăn và bong bóng "?" đọc được, và đó mới là hình đại diện bạn muốn thấy.
 * - `BeanMark` (vector) cho chỗ cần nét theo `currentColor`: sắc ở mọi tỉ lệ và
 *   chỉ 590 byte, không phụ thuộc ảnh tải được hay không.
 *
 * `BeanAvatar` dùng ảnh gốc `web/brand/bean.png` của bạn, sinh ra các file trong
 * `public/` bằng `scripts/gen-brand-assets.ts` (`pnpm brand`).
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
