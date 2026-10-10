import * as DialogPrimitive from "@radix-ui/react-dialog";
import gsap from "gsap";
import { XIcon } from "lucide-react";
import type * as React from "react";
import { useRef } from "react";

import { useGsapAnimate } from "@/lib/anim";
import { cn } from "@/lib/utils";

/**
 * Dialog — shadcn/ui trên nền `@radix-ui/react-dialog` (đã có trong
 * devDependencies từ trước nhưng chưa dùng tới).
 *
 * Overlay dùng `bg-black/60` chứ không phải token: lớp phủ luôn phải tối bất
 * kể chủ đề, và `default-src` CSP không chặn màu nền.
 */
function Dialog(props: React.ComponentProps<typeof DialogPrimitive.Root>) {
  return <DialogPrimitive.Root data-slot="dialog" {...props} />;
}

function DialogTrigger(props: React.ComponentProps<typeof DialogPrimitive.Trigger>) {
  return <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props} />;
}

function DialogClose(props: React.ComponentProps<typeof DialogPrimitive.Close>) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

function DialogOverlay({ className, ...props }: React.ComponentProps<typeof DialogPrimitive.Overlay>) {
  const ref = useRef<HTMLDivElement>(null);
  // Lớp phủ mờ dần vào thay vì nháy đen: phản hồi "màn hình đã chặn lại" mềm
  // hơn cho mắt, nhất là với hộp thoại xác nhận hành động nguy hiểm.
  useGsapAnimate(() => {
    const element = ref.current;
    if (!element) return;
    gsap.from(element, { opacity: 0, duration: 0.18, ease: "power1.out" });
  }, []);
  return (
    <DialogPrimitive.Overlay
      ref={ref}
      data-slot="dialog-overlay"
      className={cn(
        // Không dùng class `animate-in`/`fade-in` của tw-animate-css: dự án không
        // cài plugin đó, để lại chỉ là class chết. Rút gọn về hiệu ứng nền/lề
        // thuần CSS để không phụ thuộc gói mới (agents.md mục 15.10).
        "fixed inset-0 z-50 bg-black/60 transition-opacity",
        className,
      )}
      {...props}
    />
  );
}

function DialogContent({
  className,
  children,
  showClose = true,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Content> & { showClose?: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  useGsapAnimate(() => {
    const element = ref.current;
    if (!element) return;
    // Hai hướng vào cho hai hình khối: hộp thoại bung ra giữa màn hình; flyout
    // lịch sử (`data-slot="history-flyout"`) là ngăn kéo trượt từ mép trái.
    if (element.dataset.slot === "history-flyout") {
      gsap.from(element, {
        opacity: 0,
        x: -24,
        duration: 0.22,
        ease: "power2.out",
        clearProps: "opacity,transform",
      });
    } else {
      gsap.from(element, {
        opacity: 0,
        y: 10,
        scale: 0.97,
        duration: 0.22,
        ease: "back.out(1.4)",
        clearProps: "opacity,transform",
      });
    }
  }, []);
  return (
    <DialogPrimitive.Portal>
      <DialogOverlay />
      <DialogPrimitive.Content
        ref={ref}
        data-slot="dialog-content"
        className={cn(
          "fixed top-1/2 left-1/2 z-50 grid w-full max-w-lg -translate-x-1/2 -translate-y-1/2 gap-4 rounded-lg border border-border bg-popover p-6 text-popover-foreground shadow-lg",
          className,
        )}
        {...props}
      >
        {children}
        {showClose ? (
          <DialogPrimitive.Close
            className="absolute top-4 right-4 rounded-sm p-1 opacity-70 transition-opacity hover:opacity-100 focus-visible:ring-[3px] focus-visible:ring-ring/40 focus-visible:outline-none"
            aria-label="Đóng"
          >
            <XIcon className="size-4" />
          </DialogPrimitive.Close>
        ) : null}
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
}

function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-header"
      className={cn("flex flex-col gap-2 text-center sm:text-left", className)}
      {...props}
    />
  );
}

function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn("flex flex-col-reverse gap-2 sm:flex-row sm:justify-end", className)}
      {...props}
    />
  );
}

function DialogTitle({ className, ...props }: React.ComponentProps<typeof DialogPrimitive.Title>) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      className={cn("text-lg leading-none font-semibold", className)}
      {...props}
    />
  );
}

function DialogDescription({
  className,
  ...props
}: React.ComponentProps<typeof DialogPrimitive.Description>) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  );
}

export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogTitle,
  DialogTrigger,
};
