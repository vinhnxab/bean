import { cva, type VariantProps } from "class-variance-authority";
import type * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Badge — từ shadcn/ui, dùng cho **trạng thái sống** của agent.
 *
 * # Vì sao mỗi trạng thái dùng một hình khối khác nhau, không chỉ khác màu
 *
 * Màu một mình không đủ cho người mù màu (khoảng 8% nam giới) — và bảng điều
 * khiển là nơi tệ nhất để chỉ dựa vào màu. Nên: `idle` = nét, `working` = chấm
 * đặc, `awaiting_you` = chấm đặc trong viền. Có nhãn chữ đi kèm nên đọc được
 * kể cả khi không nhìn thấy màu.
 */
const badgeVariants = cva(
  "inline-flex w-fit items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs font-medium whitespace-nowrap",
  {
    variants: {
      tone: {
        /** Rảnh — trung tính, chỉ nét. */
        idle: "border-rule text-ink-muted",
        /** Đang chạy — `--live`, chấm đặc. */
        working: "border-live text-live",
        /** Đang chờ bạn duyệt — `--need`, nặng nhất vì cần hành động. */
        awaiting: "border-need bg-need/10 text-need",
        /** Nguy hiểm — `--alert`. */
        danger: "border-alert text-alert",
      },
    },
    defaultVariants: { tone: "idle" },
  },
);

function Badge({
  className,
  tone,
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return <span data-slot="badge" className={cn(badgeVariants({ tone }), className)} {...props} />;
}

/** Chấm trạng thái: hình khác nhau theo `tone` để không chỉ dựa vào màu. */
function StatusDot({
  tone,
  className,
}: {
  tone: "idle" | "working" | "awaiting" | "danger";
  className?: string;
}) {
  const base = "size-2 shrink-0";
  if (tone === "working") {
    return <span aria-hidden className={cn(base, "rounded-full bg-live", className)} />;
  }
  if (tone === "awaiting") {
    // Vòng tròn rỗng to hơn: "đang chờ" phải nổi hơn "đang chạy" về mặt hình khối.
    return <span aria-hidden className={cn(base, "rounded-full border-2 border-need", className)} />;
  }
  if (tone === "danger") {
    return <span aria-hidden className={cn(base, "rounded-sm bg-alert", className)} />;
  }
  // Rảnh: chỉ một chấm nhở màu, đúng nghĩa "không có gì đang chạy".
  return <span aria-hidden className={cn(base, "rounded-full bg-rule", className)} />;
}

export { Badge, badgeVariants, StatusDot };
