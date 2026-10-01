import { Link } from "react-router";
import { AUDIT_FEED_SIZE, useAuditFeed } from "@/features/audit/queries";
import { connectedCount, useMcpServers } from "@/features/mcp/queries";
import { useStatus } from "@/features/status/queries";
import { useTools } from "@/features/tools/queries";
import { useUsage, windowTotal } from "@/features/usage/queries";
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
  const tools = useTools();
  const mcp = useMcpServers();
  const usage = useUsage();
  const audit = useAuditFeed();
  // Cắt ở client **một lần nữa** dù đã hỏi `limit=5`: ràng buộc hiển thị là của
  // widget, không nên đặt niềm tin vào việc server tôn trọng tham số. Rẻ hơn nhiều
  // so với việc một thay đổi ở server làm vỡ bố cục bảng điều khiển.
  const entries = (audit.data?.entries ?? []).slice(0, AUDIT_FEED_SIZE);

  // Ba số dưới đây đều lấy từ endpoint **đã lọc RBAC ở server**: HUB hiện đúng
  // những gì người đang xem được phép thấy, không tự lọc lần nữa. Vì vậy phần
  // "ẩn theo role" là *thông tin* (còn tổng cộng bao nhiêu), không phải bí mật.
  const toolTotal = tools.data?.total ?? 0;
  const toolVisible = tools.data?.tools.length ?? 0;
  const toolHidden = Math.max(0, toolTotal - toolVisible);
  const mcpTotal = mcp.data?.servers.length ?? 0;
  const mcpConnected = connectedCount(mcp.data?.servers ?? []);
  const usageWindow = windowTotal(usage.data?.days ?? []);

  return (
    <div className="grid gap-4 sm:grid-cols-2">
      {/* Hoạt động: không viền, không bo — chỉ khoảng trắng phân cách. Ba vùng
          của HUB xử lý khác nhau (sơ đồ có khối, hàng chờ có viền màu, hoạt
          động phẳng) là chủ ý chống "đồng bộ card". */}
      <section aria-labelledby="hub-activity-title" data-testid="hub-activity">
        <div className="flex items-baseline justify-between gap-3">
          <h2 id="hub-activity-title" className="text-sm font-semibold">
            {t("hub.activity.title")}
          </h2>
          {entries.length > 0 ? (
            <Link to="/audit" className="text-xs text-ink-muted underline underline-offset-2 hover:text-ink">
              {t("hub.activity.viewAll")}
            </Link>
          ) : null}
        </div>
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
          {/* Số tool hiện ra đã bị RBAC cắt: ghi cả tổng để người đọc biết có
              đang bị ẩn, chứ không tưởng registry chỉ có mấy tool đó. */}
          <Row
            label={t("hub.system.tools")}
            value={tools.isPending ? "—" : toolVisible + (toolHidden > 0 ? ` / ${toolTotal}` : "")}
            numeric
            testId="hub-system-tools"
          />
          <Row
            label={t("hub.system.mcp")}
            value={mcp.isPending ? "—" : `${mcpConnected} / ${mcpTotal}`}
            numeric
            testId="hub-system-mcp"
          />
          <Row
            label={t("hub.system.usage")}
            value={usage.isPending ? "—" : usageWindow.toLocaleString()}
            numeric
            testId="hub-system-usage"
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
  testId,
}: {
  label: string;
  value: string;
  /** Định danh máy sinh ra (tên model) ⇒ mono. */
  mono?: boolean;
  /** Số đo ⇒ `tnum` của Inter, KHÔNG mono. */
  numeric?: boolean;
  testId?: string;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="text-ink-muted">{label}</dt>
      <dd data-testid={testId} className={`truncate ${mono ? "font-mono" : ""} ${numeric ? "hub-num" : ""}`}>
        {value}
      </dd>
    </div>
  );
}
