import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";
import { useI18n } from "@/i18n";
import { formatUptime } from "@/lib/format";

/**
 * Một dòng hoạt động gần đây — **rút gọn**, không phải log thô.
 *
 * Audit log đầy đủ có ở `/audit` với tham số đã redact; ở HUB chỉ hiện đủ để
 * người dùng trả lời "hệ vừa tự làm gì", và bỏ qua phần thừa. Không sao chép log
 * thô vào bảng điều khiển — đó là loại chi tiết làm bảng nặng đi mà không giúp
 * ra quyết định nào.
 */
export function ActivityFeed() {
  const { t } = useI18n();
  const status = useStatus();
  const audit = useAudit();
  const entries = (audit.data?.entries ?? []).slice(0, 5);

  return (
    <div className="grid gap-4 sm:grid-cols-2">
      {/* Hoạt động: không viền, không bo — chỉ khoảng trắng phân cách. Ba vùng
          của HUB xử lý khác nhau (sơ đồ có khối, hàng chờ có viền màu, hoạt
          động phẳng) là chủ ý chống "đồng bộ card". */}
      <section aria-labelledby="hub-activity-title" data-testid="hub-activity">
        <h2 id="hub-activity-title" className="text-sm font-semibold">
          {t("hub.activity.title")}
        </h2>
        {audit.isPending ? (
          <p className="mt-2 text-xs text-ink-muted" role="status">
            {t("common.loading")}
          </p>
        ) : entries.length === 0 ? (
          <p className="mt-2 text-xs text-ink-muted">{t("hub.activity.empty")}</p>
        ) : (
          <ul className="mt-2 flex flex-col gap-1.5">
            {entries.map((entry) => (
              // Khoá ghép từ trường có sẵn trong audit, KHÔNG dùng chỉ số mảng:
              // danh sách này thay đổi khi có dòng mới, key theo index khiến
              // React tái dùng sai node (và giữ màu trạng thái sai). `ts` + `session`
              // + `tool` là bộ định danh đủ ổn định cho một dòng audit.
              <li
                key={`${entry.ts}-${entry.session}-${entry.tool}-${entry.decided_by}`}
                className="flex gap-2 text-xs"
              >
                {/* Timestamp là dữ liệu máy sinh ra ⇒ mono. */}
                <span className="hub-num shrink-0 font-mono text-ink-muted">
                  {new Date(entry.ts).toLocaleTimeString(undefined, {
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </span>
                <span className="min-w-0 truncate">
                  <span className="font-medium">{entry.tool}</span>
                  <span className="text-ink-muted"> · {entry.decision}</span>
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section aria-labelledby="hub-system-title" data-testid="hub-system">
        <h2 id="hub-system-title" className="text-sm font-semibold">
          {t("hub.system.title")}
        </h2>
        <dl className="mt-2 flex flex-col gap-1.5 text-xs">
          <Row label={t("status.model")} value={status.data?.model ?? "—"} mono />
          <Row label={t("status.uptime")} value={formatUptime(status.data?.uptime_seconds ?? 0)} />
          <Row
            label={t("hub.system.budget")}
            value={
              status.data
                ? `${status.data.tokens_used.toLocaleString()} / ${status.data.daily_token_budget.toLocaleString()}`
                : "—"
            }
            numeric
          />
        </dl>
      </section>
    </div>
  );
}

function Row({
  label,
  value,
  mono,
  numeric,
}: {
  label: string;
  value: string;
  /** Định danh máy sinh ra (tên model) ⇒ mono. */
  mono?: boolean;
  /** Số đo ⇒ `tnum` của Inter, KHÔNG mono. */
  numeric?: boolean;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="text-ink-muted">{label}</dt>
      <dd className={`truncate ${mono ? "font-mono" : ""} ${numeric ? "hub-num" : ""}`}>{value}</dd>
    </div>
  );
}

function useStatus() {
  return useQuery({
    queryKey: queryKeys.status,
    queryFn: ({ signal }) => api.status(signal),
    refetchInterval: 30_000,
  });
}

function useAudit() {
  return useQuery({
    queryKey: queryKeys.audit,
    queryFn: ({ signal }) => api.listAudit(null, signal),
    refetchInterval: 30_000,
  });
}
