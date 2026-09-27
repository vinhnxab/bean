import * as SeparatorPrimitive from "@radix-ui/react-separator";
import type * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Đường kẻ 1px — primitive của hệ HUB.
 *
 * Ở đây đường kẻ **thay cho shadow**: chiều sâu của bảng điều khiển đến từ đường
 * kẻ, không từ đổ bóng mờ. Đó là lý do `Page.tsx` cũ (dùng `shadow-sm` ở mọi
 * khối) bị thay khi dựng HUB.
 */
function Separator({
  className,
  orientation = "horizontal",
  decorative = true,
  ...props
}: React.ComponentProps<typeof SeparatorPrimitive.Root>) {
  return (
    <SeparatorPrimitive.Root
      data-slot="separator"
      decorative={decorative}
      orientation={orientation}
      className={cn(
        "shrink-0 bg-rule",
        orientation === "horizontal" ? "h-px w-full" : "h-full w-px",
        className,
      )}
      {...props}
    />
  );
}

export { Separator };
