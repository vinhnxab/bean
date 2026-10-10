import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import gsap from "gsap";
import type * as React from "react";
import { useRef } from "react";

import { useGsapAnimate } from "@/lib/anim";
import { cn } from "@/lib/utils";

/** Tooltip — shadcn/ui trên nền Radix. Chỉ dùng cho gợi ý, không mang thông tin
 *  mà người dùng cần đọc: nội dung quan trọng phải nằm trong chính thẻ. */
function TooltipProvider({
  delayDuration = 200,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Provider>) {
  return <TooltipPrimitive.Provider delayDuration={delayDuration} {...props} />;
}

function Tooltip(props: React.ComponentProps<typeof TooltipPrimitive.Root>) {
  return <TooltipPrimitive.Root {...props} />;
}

function TooltipTrigger(props: React.ComponentProps<typeof TooltipPrimitive.Trigger>) {
  return <TooltipPrimitive.Trigger {...props} />;
}

function TooltipContent({
  className,
  sideOffset = 4,
  children,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Content>) {
  const ref = useRef<HTMLDivElement>(null);
  // Tooltip là gợi ý phụ — phải hiện gần như tức thì (110ms) để không chậm nhịp
  // rê chuột; chậm thêm một frame cũng thấy "lag" so với con trỏ.
  useGsapAnimate(() => {
    const element = ref.current;
    if (!element) return;
    gsap.from(element, { opacity: 0, duration: 0.11, ease: "power1.out" });
  }, []);
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        ref={ref}
        sideOffset={sideOffset}
        className={cn(
          "z-50 w-fit rounded-md border border-border bg-popover px-3 py-1.5 text-xs text-popover-foreground shadow-md",
          className,
        )}
        {...props}
      >
        {children}
        <TooltipPrimitive.Arrow className="fill-popover" />
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}

export { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger };
