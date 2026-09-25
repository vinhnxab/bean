import type { ReactNode } from "react";

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
        <p className="mt-2 max-w-3xl text-sm text-slate-500 dark:text-slate-400">{description}</p>
      </header>
      {children}
    </section>
  );
}

export function LoadingState({ label }: { label: string }) {
  return (
    <div
      className="rounded-2xl border border-slate-200 bg-white p-6 text-sm text-slate-500 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-400"
      role="status"
    >
      {label}
    </div>
  );
}

export function ErrorState({ message, onRetry }: { message?: string; onRetry?: () => void }) {
  const { t } = useI18n();
  return (
    <div
      className="rounded-2xl border border-rose-200 bg-rose-50 p-5 text-sm text-rose-800 dark:border-rose-900 dark:bg-rose-950 dark:text-rose-200"
      role="alert"
    >
      <p>{message ?? t("common.error")}</p>
      {onRetry ? (
        <button
          type="button"
          onClick={onRetry}
          className="mt-3 rounded-lg border border-rose-400 px-3 py-1.5 font-semibold hover:bg-rose-100 dark:hover:bg-rose-900"
        >
          {t("common.retry")}
        </button>
      ) : null}
    </div>
  );
}

export function EmptyState({ title, description }: { title: string; description: string }) {
  return (
    <div className="rounded-2xl border border-dashed border-slate-300 bg-white p-8 text-center dark:border-slate-700 dark:bg-slate-900">
      <p className="font-semibold">{title}</p>
      <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">{description}</p>
    </div>
  );
}

export function Panel({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <section
      className={`rounded-2xl border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-800 dark:bg-slate-900 sm:p-5 ${className}`}
    >
      {children}
    </section>
  );
}
