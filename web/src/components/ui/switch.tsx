import type * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Switch (toggle) — tự viết, không thêm dependency Radix.
 *
 * # Vì sao không dùng `@radix-ui/react-switch`
 *
 * Chỉ cần một phần tử `role="switch"` + `aria-checked`: thêm một package cho một
 * nút gạt là chi phí supply-chain không đáng (mục 15.10). Phần tử `<button>` sẵn
 * nhận focus/Enter/Space đúng cách, còn hình dạng là CSS thuần.
 *
 * # Token màu
 *
 * Bật = `bg-brand` (hành động), tắt = `bg-rule` (chỉ là vạch phân cách, không phải
 * trạng thái). Thumb dùng `--brand-ink`/`--ink-muted` — chữ/nét đặt trên nền tương
 * ứng, nên hai chủ đề sáng/tối đều đúng tương phản mà không cần viết màu thô.
 *
 * # Canh thumb
 *
 * Track `p-0.5` (2px) + thumb `size-3.5` (14px): 2×1 viền + 2×2 đệm + 14 = 20px
 * khít chiều cao; khi bật dịch `translate-x-4` (16px) để thumb cách mép phải
 * đúng 2px — đối xứng với khi tắt (cách mép trái 2px). Bản cũ `p-[3px]` +
 * `translate-x-[14px]` làm thumb tràn hết đệm phải, dính sát viền khi bật.
 */
function Switch({
  checked,
  onCheckedChange,
  disabled = false,
  className,
  ...props
}: Omit<React.ComponentProps<"button">, "onChange" | "checked"> & {
  checked: boolean;
  onCheckedChange?: (checked: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onCheckedChange?.(!checked)}
      className={cn(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border p-0.5 transition-colors",
        "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
        checked ? "border-brand bg-brand" : "border-rule bg-rule/60",
        disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer",
        className,
      )}
      {...props}
    >
      <span
        aria-hidden="true"
        className={cn(
          "pointer-events-none block size-3.5 rounded-full transition-transform",
          checked ? "translate-x-4 bg-brand-ink" : "translate-x-0 bg-ink-muted",
        )}
      />
    </button>
  );
}

export { Switch };
