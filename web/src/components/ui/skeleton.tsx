import type * as React from "react";

import { cn } from "@/lib/utils";

/** Skeleton — chỗ đang tải. Dùng thay cho nhấp nháy chữ "Đang tải…" khi biết
 *  trước hình dạng (danh sách, thẻ), giúp không dồn layout khi dữ liệu về. */
function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="skeleton"
      className={cn("animate-pulse rounded-md bg-surface-hover", className)}
      {...props}
    />
  );
}

export { Skeleton };
