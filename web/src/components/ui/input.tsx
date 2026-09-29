import type * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Input — shadcn/ui, chỉ dùng token nên tự đúng màu ở cả hai chủ đề.
 * `.field-input` trong index.css là lớp tương thích cho các chỗ còn dùng class
 * cũ; dần dần các chỗ đó chuyển sang đây.
 */
function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-[color,box-shadow] outline-none",
        "placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground",
        "focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/40",
        "aria-invalid:border-destructive aria-invalid:ring-destructive/25",
        "disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

export { Input };
