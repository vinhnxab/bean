import type { ReactNode } from "react";

import { Skeleton } from "@/components/ui/skeleton";
import { useI18n } from "@/i18n";

export function PageShell({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <section
      className="mx-auto w-full max-w-7xl flex-1 px-4 py-6 pb-24 pt-20 sm:px-6 md:pb-10 md:pt-8"
      aria-labelledby="page-title"
    >
      <header className="mb-6">
        <h1 id="page-title" className="text-2xl font-bold tracking-tight sm:text-3xl">
          {title}
        </h1>
        <p className="mt-2 max-w-3xl text-sm text-muted-foreground">{description}</p>
      </header>
      {children}
    </section>
  );
}

export function LoadingState({ label }: { label: string }) {
  return (
    <div className="rounded-lg border border-border bg-card p-6 text-sm text-muted-foreground" role="status">
      {label}
    </div>
  );
}

/**
 * Khung chờ dùng cho `Suspense` khi route đang tải chunk (mục: code-splitting).
 *
 * Vì sao skeleton chứ không phải `LoadingState`
 *
 * `LoadingState` là một hộp chữ "Đang tải…" nằm giữa trang. Ở đây cấu trúc trang
 * đã biết trước (tiêu đề + khối nội dung) nên vẽ skeleton đúng hình dạng: không
 * nhảy layout khi chunk tới, và màn hình đọc vẫn có `role="status"` để báo trạng
 * thái. Hộp skeleton có `aria-hidden` vì nó **không mang thông tin** — thứ duy
 * nhất người đọc màn hình cần là "đang tải", đã nói bằng `role="status"`.
 */
export function RouteFallback() {
  return (
    <div
      className="mx-auto w-full max-w-7xl flex-1 px-4 py-6 pb-24 pt-20 sm:px-6 md:pb-10 md:pt-8"
      role="status"
    >
      <div aria-hidden="true" className="mb-6">
        <Skeleton className="h-8 w-40" />
        <Skeleton className="mt-2 h-4 w-72 max-w-full" />
      </div>
      <div aria-hidden="true" className="grid gap-4 lg:grid-cols-3">
        <Skeleton className="h-64 lg:col-span-2" />
        <Skeleton className="h-64" />
      </div>
    </div>
  );
}

export function ErrorState({ message, onRetry }: { message?: string; onRetry?: () => void }) {
  const { t } = useI18n();
  return (
    <div
      className="rounded-lg border border-destructive/40 bg-destructive/10 p-5 text-sm text-destructive"
      role="alert"
    >
      <p>{message ?? t("common.error")}</p>
      {onRetry ? (
        <button
          type="button"
          onClick={onRetry}
          className="mt-3 rounded-lg border border-destructive/40 px-3 py-1.5 font-semibold hover:bg-destructive/20"
        >
          {t("common.retry")}
        </button>
      ) : null}
    </div>
  );
}

export function EmptyState({ title, description }: { title: string; description: string }) {
  return (
    <div className="rounded-lg border border-dashed border-border bg-card p-8 text-center">
      <p className="font-semibold">{title}</p>
      <p className="mt-1 text-sm text-muted-foreground">{description}</p>
    </div>
  );
}

export function Panel({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <section className={`rounded-lg border border-border bg-card p-4 shadow-sm sm:p-5 ${className}`}>
      {children}
    </section>
  );
}
