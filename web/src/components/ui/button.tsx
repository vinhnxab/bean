import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import gsap from "gsap";
import type * as React from "react";
import { useRef } from "react";

import { useGsapAnimate } from "@/lib/anim";
import { cn } from "@/lib/utils";

/**
 * Nút — lấy từ shadcn/ui, chỉ đổi *token* theo hệ HUB, không đổi cấu trúc.
 *
 * Không có shadow và bo góc nhỏ (`rounded-md` = 5px từ `--radius`) là chủ ý:
 * đó là phép phân biệt giữa "bản vẽ kỹ thuật" và "thẻ SaaS".
 */
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors disabled:pointer-events-none disabled:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:opacity-90",
        outline: "border border-border bg-transparent hover:bg-accent",
        ghost: "hover:bg-accent",
        /** Hành động cần người dùng chủ động (Duyệt) — dùng `--need`. */
        attention: "border border-need text-need hover:bg-need-strong hover:text-surface",
        /** Hành động nguy hiểm — dùng `--alert`. */
        danger: "bg-destructive text-destructive-foreground hover:opacity-90",
        link: "text-ink underline underline-offset-4 hover:opacity-80",
      },
      size: {
        sm: "h-8 px-3 text-sm",
        default: "h-9 px-4",
        lg: "h-10 px-5",
        icon: "size-9",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
);

function Button({
  className,
  variant,
  size,
  asChild = false,
  ...props
}: React.ComponentProps<"button"> & VariantProps<typeof buttonVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot : "button";
  const ref = useRef<HTMLButtonElement>(null);

  // Cảm giác bấm: thu nhẹ khi nhấn, nảy về khi thả — thứ `transition-colors`
  // không cho được và là phản hồi xúc giác duy nhất của giao diện web. Nút
  // `disabled` có `pointer-events-none` nên tự loại khỏi hiệu ứng này.
  useGsapAnimate(() => {
    const element = ref.current;
    if (!element) return;
    const scaleTo = (value: number) =>
      gsap.to(element, {
        scale: value,
        duration: value === 1 ? 0.25 : 0.12,
        ease: value === 1 ? "back.out(2)" : "power2.out",
        overwrite: true,
        ...(value === 1 ? { clearProps: "transform" } : {}),
      });
    const down = () => {
      scaleTo(0.96);
    };
    const up = () => {
      scaleTo(1);
    };
    element.addEventListener("pointerdown", down);
    element.addEventListener("pointerup", up);
    element.addEventListener("pointerleave", up);
    element.addEventListener("pointercancel", up);
  }, []);

  return (
    <Comp
      ref={ref}
      data-slot="button"
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  );
}

export { Button, buttonVariants };
