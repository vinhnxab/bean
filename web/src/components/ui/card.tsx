import type * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Card — shadcn/ui. Thay cho các `<section className="rounded-lg border * border-border bg-card">` viết tay trước đây, nên bề mặt và bo góc luôn
 * đúng token ở cả hai chủ đề.
 */
function Card({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card"
      className={cn(
        "flex flex-col gap-6 rounded-lg border border-border bg-card py-6 text-card-foreground shadow-sm",
        className,
      )}
      {...props}
    />
  );
}

function CardHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-header"
      className={cn("grid auto-rows-min items-start gap-1.5 px-6", className)}
      {...props}
    />
  );
}

/**
 * Tiêu đề thẻ — render `h3`, KHÔNG phải `div` như shadcn gốc.
 *
 * Ở LoginPage, `CardTitle` đóng vai tiêu đề trang. Nếu nó là `div` thì mất
 * luôn vai trò heading: trình đọc màn hình không còn "báo tên trang", và
 * `getByRole("heading")` không tìm thấy. Một thẻ luôn nằm trong vùng đã có
 * tiêu đề cấp cao hơn, nên `h3` là cấp đúng.
 */
function CardTitle({ className, ...props }: React.ComponentProps<"h3">) {
  return <h3 data-slot="card-title" className={cn("font-semibold leading-none", className)} {...props} />;
}

function CardDescription({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div data-slot="card-description" className={cn("text-sm text-muted-foreground", className)} {...props} />
  );
}

function CardContent({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-content" className={cn("px-6", className)} {...props} />;
}

function CardFooter({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="card-footer" className={cn("flex items-center px-6", className)} {...props} />;
}

export { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle };
