import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import type * as React from "react";

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
  return <Comp data-slot="button" className={cn(buttonVariants({ variant, size }), className)} {...props} />;
}

export { Button, buttonVariants };
