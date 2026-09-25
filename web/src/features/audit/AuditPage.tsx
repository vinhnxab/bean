import { EmptyState, ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import { useAuditLog } from "@/features/audit/queries";
import { useI18n } from "@/i18n";
import { formatDateTime, formatJson } from "@/lib/format";

export function AuditPage() {
  const { t } = useI18n();
  const audit = useAuditLog();
  const entries = audit.data?.pages.flatMap((page) => page.entries) ?? [];

  return (
    <PageShell title={t("audit.title")} description={t("audit.description")}>
      <Panel>
        {audit.isPending ? <LoadingState label={t("common.loading")} /> : null}
        {audit.isError ? <ErrorState onRetry={() => void audit.refetch()} /> : null}
        {audit.data && entries.length === 0 ? (
          <EmptyState title={t("audit.empty")} description={t("audit.emptyDescription")} />
        ) : null}
        {entries.length > 0 ? (
          <div className="overflow-x-auto">
            <table className="w-full min-w-[56rem] border-collapse text-left text-sm">
              <thead>
                <tr className="border-b border-slate-200 text-xs uppercase tracking-wide text-slate-500 dark:border-slate-700 dark:text-slate-400">
                  <th className="px-3 py-3">{t("audit.time")}</th>
                  <th className="px-3 py-3">{t("audit.channel")}</th>
                  <th className="px-3 py-3">{t("audit.tool")}</th>
                  <th className="px-3 py-3">{t("audit.result")}</th>
                  <th className="px-3 py-3">{t("audit.actor")}</th>
                  <th className="px-3 py-3">{t("audit.args")}</th>
                </tr>
              </thead>
              <tbody>
                {entries.map((entry) => (
                  <tr
                    key={`${entry.ts}-${entry.session}-${entry.channel}-${entry.tool}`}
                    className="border-b border-slate-100 align-top dark:border-slate-800"
                  >
                    <td className="whitespace-nowrap px-3 py-3">
                      <time dateTime={entry.ts}>{formatDateTime(entry.ts)}</time>
                    </td>
                    <td className="px-3 py-3">{entry.channel}</td>
                    <td className="px-3 py-3 font-mono text-xs">{entry.tool}</td>
                    <td className="px-3 py-3">
                      <span
                        className={
                          entry.ok === false
                            ? "text-rose-600 dark:text-rose-400"
                            : "text-emerald-700 dark:text-emerald-300"
                        }
                      >
                        {entry.ok === null
                          ? t("audit.pending")
                          : entry.ok
                            ? t("audit.ok")
                            : t("audit.failed")}
                      </span>
                      {entry.error ? (
                        <p className="mt-1 max-w-xs text-xs text-rose-600">{entry.error}</p>
                      ) : null}
                    </td>
                    <td className="px-3 py-3 text-xs">{entry.decided_by}</td>
                    <td className="px-3 py-3">
                      <details>
                        <summary className="cursor-pointer text-xs font-semibold text-emerald-700 dark:text-emerald-300">
                          {t("audit.viewArgs")}
                        </summary>
                        <pre className="mt-2 max-h-40 max-w-sm overflow-auto whitespace-pre-wrap break-words rounded-lg bg-slate-100 p-2 font-mono text-xs dark:bg-slate-950">
                          {formatJson(entry.args)}
                        </pre>
                      </details>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
        {audit.hasNextPage ? (
          <div className="mt-4 flex justify-center">
            <button
              type="button"
              onClick={() => void audit.fetchNextPage()}
              disabled={audit.isFetchingNextPage}
              className="rounded-lg border border-slate-300 px-4 py-2 text-sm font-semibold hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-800"
            >
              {audit.isFetchingNextPage ? t("common.loading") : t("audit.loadMore")}
            </button>
          </div>
        ) : null}
      </Panel>
    </PageShell>
  );
}
