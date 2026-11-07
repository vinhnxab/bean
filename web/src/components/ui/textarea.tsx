import type * as React from "react";

import { cn } from "@/lib/utils";

/** Textarea — dùng cho ô soạn thảo (ghi nhớ, mô tả skill, ghi chú tác vụ). */
function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "flex min-h-20 w-full rounded-lg border border-border bg-transparent px-3 py-2 text-sm shadow-xs transition-[color,box-shadow] outline-none",
        "placeholder:text-muted-foreground field-sizing-content",
        "focus:border-live focus:ring-2 focus:ring-live/30",
        "aria-invalid:border-destructive aria-invalid:ring-destructive/25",
        "disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

export { Textarea };
