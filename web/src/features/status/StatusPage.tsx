import { ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import { useStatus } from "@/features/status/queries";
import { useI18n } from "@/i18n";
import { formatUptime } from "@/lib/format";

export function StatusPage() {
  const { t } = useI18n();
  const status = useStatus();
  const data = status.data;
  const percent =
    data && data.daily_token_budget > 0
      ? Math.min(100, Math.round((data.tokens_used / data.daily_token_budget) * 100))
      : 0;
  const overBudget = data !== undefined && data.tokens_used > data.daily_token_budget;

  return (
    <PageShell title={t("status.title")} description={t("status.description")}>
      {status.isPending ? <LoadingState label={t("common.loading")} /> : null}
      {status.isError ? <ErrorState onRetry={() => void status.refetch()} /> : null}
      {data ? (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
          <StatusCard label={t("status.version")} value={data.version} />
          <StatusCard label={t("status.model")} value={data.model} />
          <StatusCard label={t("status.maxSteps")} value={data.max_steps.toLocaleString()} />
          <StatusCard label={t("status.uptime")} value={formatUptime(data.uptime_seconds)} />
          <Panel className="sm:col-span-2 xl:col-span-3">
            <div className="flex flex-wrap items-end justify-between gap-3">
              <div>
                <p className="text-sm font-medium text-slate-500 dark:text-slate-400">
                  {t("status.tokenUsage")}
                </p>
                <p className="mt-1 text-2xl font-bold">
                  {data.tokens_used.toLocaleString()} / {data.daily_token_budget.toLocaleString()}
                </p>
              </div>
              <span
                className={
                  overBudget
                    ? "text-sm font-semibold text-rose-600"
                    : "text-sm text-slate-500 dark:text-slate-400"
                }
              >
                {overBudget ? t("status.budgetExceeded") : `${percent}%`}
              </span>
            </div>
            <div
              className="mt-3 h-3 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800"
              role="progressbar"
              aria-valuenow={percent}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-label={t("status.tokenUsage")}
            >
              <div
                className={
                  overBudget ? "h-full rounded-full bg-rose-600" : "h-full rounded-full bg-emerald-600"
                }
                style={{ width: `${percent}%` }}
              />
            </div>
          </Panel>
          <Panel className="sm:col-span-2 xl:col-span-3">
            <h2 className="font-semibold">{t("status.channels")}</h2>
            {data.channels.length > 0 ? (
              <ul className="mt-3 flex flex-wrap gap-2">
                {data.channels.map((channel) => (
                  <li
                    key={channel}
                    className="rounded-full bg-emerald-100 px-3 py-1 text-sm font-medium text-emerald-800 dark:bg-emerald-950 dark:text-emerald-200"
                  >
                    {channel}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="mt-2 text-sm text-slate-500">{t("status.noChannels")}</p>
            )}
          </Panel>
        </div>
      ) : null}
    </PageShell>
  );
}

function StatusCard({ label, value }: { label: string; value: string }) {
  return (
    <Panel>
      <p className="text-sm text-slate-500 dark:text-slate-400">{label}</p>
      <p className="mt-2 break-words text-xl font-bold">{value}</p>
    </Panel>
  );
}
