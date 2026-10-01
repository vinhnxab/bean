import type { SystemResponse } from "@/api/bindings";
import { ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import { useStatus } from "@/features/status/queries";
import { useSystem } from "@/features/system/queries";
import { useUsage } from "@/features/usage/queries";
import { UsageBars } from "@/features/usage/UsageBars";
import { useI18n } from "@/i18n";
import { formatUptime } from "@/lib/format";

/**
 * Màn **Trạng thái** — ba lớp thông tin khác nhau, đặt cạnh nhau nhưng không trộn:
 *
 * * `/api/status` — nhịp sống (phiên bản, model, token hôm nay, uptime, kênh đang chạy)
 * * `/api/usage` — xu hướng 14 ngày, thứ mà một con số hôm nay không kể được
 * * `/api/system` — cấu hình vận hành (sandbox, kênh, role), **đã khử secret**
 *
 * # Vì sao model/budget không lặp lại ở panel cấu hình
 *
 * Một sự thật, một chỗ hiển thị. Lặp lại nghĩa là hai nơi có thể lệch nhau khi API
 * đổi, và người dùng không biết ô nào đúng.
 */
export function StatusPage() {
  const { t } = useI18n();
  const status = useStatus();
  const usage = useUsage();
  const system = useSystem();
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
                <p className="text-sm font-medium text-muted-foreground">{t("status.tokenUsage")}</p>
                <p className="mt-1 text-2xl font-bold">
                  {data.tokens_used.toLocaleString()} / {data.daily_token_budget.toLocaleString()}
                </p>
              </div>
              <span
                className={
                  overBudget ? "text-sm font-semibold text-destructive" : "text-sm text-muted-foreground"
                }
              >
                {overBudget ? t("status.budgetExceeded") : `${percent}%`}
              </span>
            </div>
            <div
              className="mt-3 h-3 overflow-hidden rounded-full bg-accent"
              role="progressbar"
              aria-valuenow={percent}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-label={t("status.tokenUsage")}
            >
              <div
                className={overBudget ? "h-full rounded-full bg-destructive" : "h-full rounded-full bg-live"}
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
                    className="rounded-full bg-live/10 px-3 py-1 text-sm font-medium text-live"
                  >
                    {channel}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="mt-2 text-sm text-muted-foreground">{t("status.noChannels")}</p>
            )}
          </Panel>
        </div>
      ) : null}

      {usage.data ? (
        <div className="mt-4">
          <UsageBars days={usage.data.days} budget={usage.data.daily_token_budget} />
        </div>
      ) : null}

      {system.data ? (
        <div className="mt-4">
          <SystemPanel system={system.data} />
        </div>
      ) : null}
    </PageShell>
  );
}

function StatusCard({ label, value }: { label: string; value: string }) {
  return (
    <Panel>
      <p className="text-sm text-muted-foreground">{label}</p>
      <p className="mt-2 break-words text-xl font-bold">{value}</p>
    </Panel>
  );
}

/**
 * Cấu hình vận hành từ `/api/system`.
 *
 * Panel này tồn tại để trả lời câu hỏi mà log mới trả lời được: "lệnh shell có đang
 * chạy trong sandbox không, có mạng không, ai được vào". Vì dữ liệu đã được server
 * khử secret, ở đây **không có** chỗ nào để vô tình in API key hay bot token — và
 * cũng không có lý do gì để thêm vào.
 */
function SystemPanel({ system }: { system: SystemResponse }) {
  const { t } = useI18n();
  const yes = t("system.on");
  const no = t("system.off");

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Panel>
        <h2 className="font-semibold">{t("system.sandbox")}</h2>
        <dl className="mt-3 space-y-1.5 text-sm">
          <Row
            label={t("system.sandboxMode")}
            value={
              system.sandbox.mode === "host"
                ? `${system.sandbox.mode} · ${t("system.hostWarning")}`
                : system.sandbox.mode
            }
            mono
          />
          <Row label={t("system.sandboxNetwork")} value={system.sandbox.network ? yes : no} />
          <Row label={t("system.image")} value={system.sandbox.image} mono />
          <Row
            label={t("system.limits")}
            value={`${system.sandbox.memory} · ${system.sandbox.cpus} CPU · ${system.sandbox.pids_limit} proc`}
          />
          <Row
            label={t("system.commandTimeout")}
            value={t("system.seconds", { seconds: system.sandbox.timeout_seconds })}
          />
        </dl>
      </Panel>

      <Panel>
        <h2 className="font-semibold">{t("system.channels")}</h2>
        <dl className="mt-3 space-y-1.5 text-sm">
          <Row label={t("system.web")} value={system.web.enabled ? system.web.bind : no} mono />
          <Row
            label={t("system.allowRemote")}
            value={system.web.allow_remote ? `${yes} · ${t("system.allowRemoteHint")}` : no}
          />
          <Row
            label={t("system.sessionTtl")}
            value={t("system.hours", { hours: system.web.session_ttl_hours })}
          />
          <Row
            label={t("system.telegram")}
            value={
              system.telegram.enabled
                ? t("system.telegramUsers", { count: system.telegram.allowed_users })
                : no
            }
          />
          <Row label={t("system.timezone")} value={system.timezone} mono />
          <Row label={t("system.provider")} value={system.provider} mono />
          <Row label={t("system.apiKeyEnv")} value={system.api_key_env} mono />
        </dl>
      </Panel>

      <Panel className="lg:col-span-2">
        <h2 className="font-semibold">{t("system.roles")}</h2>
        {system.rbac_enabled ? (
          <ul className="mt-3 space-y-2">
            {system.roles.map((role) => (
              <li key={role.name} className="flex flex-wrap items-center gap-2 text-sm">
                <span className="font-mono font-semibold">{role.name}</span>
                {role.tool_tags.map((tag) => (
                  <Chip key={tag} text={tag} />
                ))}
                {role.forbid_tags.map((tag) => (
                  <Chip key={`forbid-${tag}`} text={`¬${tag}`} alert />
                ))}
              </li>
            ))}
          </ul>
        ) : (
          <p className="mt-2 text-sm text-muted-foreground">{t("system.rbacOff")}</p>
        )}

        <h3 className="mt-5 font-semibold">{t("system.projects")}</h3>
        <div className="mt-2 flex flex-wrap gap-1.5">
          {system.projects.map((project) => (
            <Chip key={project} text={project} />
          ))}
        </div>

        <h3 className="mt-5 font-semibold">{t("system.toolGroups")}</h3>
        <div className="mt-2 flex flex-wrap gap-1.5">
          {system.tool_groups.map((group) => (
            <Chip key={group} text={group} />
          ))}
        </div>

        <h3 className="mt-5 font-semibold">{t("system.features")}</h3>
        <dl className="mt-2 grid gap-1.5 text-sm sm:grid-cols-2">
          <Row label={t("system.learning")} value={system.learning_enabled ? yes : no} />
          <Row label={t("system.mcpServer")} value={system.mcp_server_enabled ? yes : no} />
          <Row label={t("system.browser")} value={system.browser_enabled ? yes : no} />
          <Row label={t("system.mcpClients")} value={system.mcp_clients.toLocaleString()} />
        </dl>
      </Panel>
    </div>
  );
}

function Row({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex flex-wrap items-baseline justify-between gap-2">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className={mono ? "text-right font-mono text-xs break-all" : "text-right font-medium"}>{value}</dd>
    </div>
  );
}

function Chip({ text, alert = false }: { text: string; alert?: boolean }) {
  return (
    <span
      className={
        alert
          ? "rounded-md border border-alert/50 px-2 py-0.5 font-mono text-xs text-alert"
          : "rounded-md border border-rule px-2 py-0.5 font-mono text-xs text-muted-foreground"
      }
    >
      {text}
    </span>
  );
}
