import * as PopoverPrimitive from "@radix-ui/react-popover";
import gsap from "gsap";
import type * as React from "react";
import { useRef } from "react";

import { useGsapAnimate } from "@/lib/anim";
import { cn } from "@/lib/utils";

/**
 * Popover — shadcn/ui trên nền `@radix-ui/react-popover`: tấm nội dung nổi được
 * neo vào trigger và tự lật/né mép màn hình qua popper, khác Dialog (giữa màn
 * hình, chặn cả trang) — đúng cho pattern "dấu hỏi mở giải thích kế bên nó".
 *
 * Không `modal` mặc định: popover giải thích không cướp điều khiển — người dùng
 * vẫn bấm được chỗ khác (bấm ra ngoài tự đóng qua DismissableLayer của Radix).
 *
 * Animation bằng gsap như `dialog.tsx` — project không cài tw-animate-css, để
 * class `animate-in` chỉ là class chết (xem comment dialog.tsx).
 */
function Popover(props: React.ComponentProps<typeof PopoverPrimitive.Root>) {
  return <PopoverPrimitive.Root data-slot="popover" {...props} />;
}

function PopoverTrigger(props: React.ComponentProps<typeof PopoverPrimitive.Trigger>) {
  return <PopoverPrimitive.Trigger data-slot="popover-trigger" {...props} />;
}

function PopoverContent({
  className,
  children,
  sideOffset = 8,
  align = "start",
  collisionPadding = 16,
  ...props
}: React.ComponentProps<typeof PopoverPrimitive.Content>) {
  const ref = useRef<HTMLDivElement>(null);
  // Nhanh hơn dialog (0.16s so với 0.22s): popover giải thích phải cảm giác "nhẹ",
  // chậm thêm một chút là thành chướng ngại giữa mắt và nội dung đang đọc.
  useGsapAnimate(() => {
    const element = ref.current;
    if (!element) return;
    gsap.from(element, {
      opacity: 0,
      y: 6,
      scale: 0.97,
      duration: 0.16,
      ease: "power1.out",
      clearProps: "opacity,transform",
    });
  }, []);
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        ref={ref}
        data-slot="popover-content"
        sideOffset={sideOffset}
        align={align}
        collisionPadding={collisionPadding}
        className={cn(
          "glass z-50 w-72 rounded-xl border border-border p-4 text-popover-foreground",
          className,
        )}
        {...props}
      >
        {children}
      </PopoverPrimitive.Content>
    </PopoverPrimitive.Portal>
  );
}

export { Popover, PopoverContent, PopoverTrigger };
