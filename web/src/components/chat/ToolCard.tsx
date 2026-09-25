import { useState } from "react";

import { api } from "@/api/client";
import { useI18n } from "@/i18n";

export type ToolCardStatus = "running" | "ok" | "error";

export function ToolCard({
  name,
  summary,
  argsPreview,
  outputPreview,
  status,
  messageId,
}: {
  name: string;
  summary: string;
  argsPreview: string;
  outputPreview: string;
  status: ToolCardStatus;
  messageId?: number;
}) {
  const { t } = useI18n();
  const [full, setFull] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [showFull, setShowFull] = useState(false);
  const statusText =
    status === "running" ? t("tool.running") : status === "ok" ? t("tool.ok") : t("tool.error");

  async function openFull() {
    if (!messageId || full !== null) {
      setShowFull(true);
      return;
    }
    setLoading(true);
    try {
      const dto = await api.getMessage(messageId);
      const message = dto.message;
      setFull(
        typeof message === "object" &&
          message !== null &&
          !Array.isArray(message) &&
          typeof message.text === "string"
          ? message.text
          : "",
      );
    } catch {
      setFull(t("chat.historyError"));
    } finally {
      setLoading(false);
      setShowFull(true);
    }
  }

  return (
    <article
      className="rounded-2xl border border-slate-200 bg-white p-3 shadow-sm dark:border-slate-800 dark:bg-slate-900"
      aria-label={name}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <h3 className="truncate font-mono text-sm font-semibold">{name}</h3>
          <p className="truncate text-sm text-slate-500 dark:text-slate-400">{summary}</p>
        </div>
        <span
          className={`rounded-full px-2 py-1 text-xs font-semibold ${status === "error" ? "bg-rose-100 text-rose-700 dark:bg-rose-950 dark:text-rose-300" : status === "ok" ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300" : "bg-amber-100 text-amber-700 dark:bg-amber-950 dark:text-amber-300"}`}
        >
          {statusText}
        </span>
      </div>
      <details className="mt-3">
        <summary className="cursor-pointer text-sm font-medium">{t("tool.args")}</summary>
        <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-slate-100 p-3 font-mono text-xs dark:bg-slate-950">
          {argsPreview}
        </pre>
      </details>
      <div className="mt-3">
        <p className="mb-1 text-sm font-medium">{t("tool.output")}</p>
        <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-slate-950 p-3 font-mono text-xs leading-5 text-slate-100">
          {outputPreview || t("tool.noOutput")}
        </pre>
      </div>
      {messageId ? (
        <button
          type="button"
          onClick={openFull}
          disabled={loading}
          className="mt-2 text-sm font-semibold text-emerald-700 hover:underline disabled:opacity-50 dark:text-emerald-300"
        >
          {loading ? t("common.loading") : t("tool.viewFull")}
        </button>
      ) : null}
      {showFull ? (
        <div
          className="mt-3 rounded-xl border border-slate-200 p-3 dark:border-slate-700"
          role="dialog"
          aria-label={t("tool.fullTitle")}
        >
          <div className="mb-2 flex justify-end">
            <button
              type="button"
              onClick={() => setShowFull(false)}
              className="text-sm text-slate-500 hover:text-slate-900 dark:hover:text-white"
            >
              {t("common.close")}
            </button>
          </div>
          <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words font-mono text-xs">
            {full ?? ""}
          </pre>
        </div>
      ) : null}
    </article>
  );
}
