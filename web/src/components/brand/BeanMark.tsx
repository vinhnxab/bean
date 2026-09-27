import { MARK_STROKE, MARK_VIEWBOX, type MarkVariant, markPaths } from "@/components/brand/markPaths";
import { cn } from "@/lib/utils";

/**
 * Mascot Bean (poodle) — component dùng chung cho **mọi** vị trí hiển thị.
 *
 * 5 vị trí: favicon (`public/favicon.svg`, sinh từ `markPaths`), logo HUB (44px),
 * avatar Manager trong chat (28px), skeleton loading (24px), trạng thái rỗng (56px).
 *
 * Dùng `currentColor` nên mascot tự nhận màu của vùng chứa — đó là lý do "chó poodle"
 * là **thứ ấm duy nhất** trên màn hình mà vẫn không phải một khối màu trang trí.
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
