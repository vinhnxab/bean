import { XIcon } from "lucide-react";
import { Link } from "react-router";
import { AUDIT_FEED_SIZE, useAuditFeed } from "@/features/audit/queries";
import { connectedCount, useMcpServers } from "@/features/mcp/queries";
import { useStatus } from "@/features/status/queries";
import { useTools } from "@/features/tools/queries";
import { useUsage, windowTotal } from "@/features/usage/queries";
import { useI18n } from "@/i18n";
import { formatDay, formatUptime } from "@/lib/format";

/**
 * Sidebar phải của Dashboard — đảo kính thứ ba kiểu Telegram Web, đối trọng với
 * `RightSidebar` của trang chat. Chứa ba panel hologram xếp dọc:
 *
 * 1. Token — hôm nay so với ngân sách + xu hướng 14 ngày
 * 2. Hoạt động gần đây (audit, rút gọn)
 * 3. Hệ thống — version, model, uptime, kênh, tool, MCP (toàn bộ số liệu sống
 *    của tiến trình, gom về đúng một nơi)
 *
 * # Vì sao toàn bộ số liệu nằm ở đây
 *
 * Nội dung chính của Dashboard giữ đúng việc cần HÀNH ĐỘNG (sơ đồ hệ thống,
 * hàng chờ duyệt); số liệu đo lường là ngữ cảnh theo dõi — đọc ở cột bên, không
 * cạnh tranh sự chú ý. Provider/endpoint/key không lặp ở đây nữa: pill trạng
 * thái model ở sidebar trái đã là nơi hiển thị duy nhất (một sự thật, một chỗ).
 *
 * # Hai trạng thái, một component
 *
 * Cùng pattern với `RightSidebar`: desktop (`md:` trở lên) là đảo kính thường
 * trú luôn mở; mobile là ngăn kéo trượt từ mép phải, điều khiển bằng
 * `open`/`onClose` qua icon trên `TopNav`. Component **tự fetch** dữ liệu —
 * ChatLayout truyền đúng hai prop điều khiển, không luồn state qua giữa.
 */
export function DashboardSidebar({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t, lang } = useI18n();
  const status = useStatus();
  const usage = useUsage();
  const tools = useTools();
  const mcp = useMcpServers();
  const audit = useAuditFeed();

  // Token hôm nay so với ngân sách ngày — nguồn `/api/status`, cùng dữ liệu màn Status.
  const used = status.data?.tokens_used;
  const budget = status.data?.daily_token_budget ?? 0;
  const percent = used !== undefined && budget > 0 ? Math.min(100, Math.round((used / budget) * 100)) : 0;
  const over = used !== undefined && budget > 0 && used > budget;

  // Xu hướng 14 ngày — cột cao theo tỷ lệ ngày cao nhất của cửa sổ.
  const days = usage.data?.days ?? [];
  const trendReady = usage.isSuccess && days.length > 0;
  const total = windowTotal(days);
  const peak = trendReady
    ? days.reduce((best, day) => (day.total_tokens > best.total_tokens ? day : best), days[0])
    : null;
  const scale = peak && peak.total_tokens > 0 ? peak.total_tokens : 1;

  // Audit: cắt ở client lần nữa dù đã hỏi `limit=5` — ràng buộc hiển thị là của
  // widget, không đặt niềm tin vào việc server tôn trọng tham số.
  const entries = (audit.data?.entries ?? []).slice(0, AUDIT_FEED_SIZE);

  // Số tool/MCP hiện ra đã bị RBAC lọc ở server: ghi cả tổng để người đọc biết
  // có đang bị ẩn, chứ không tưởng registry chỉ có mấy cái đó.
  const toolTotal = tools.data?.total ?? 0;
  const toolVisible = tools.data?.tools.length ?? 0;
  const toolHidden = Math.max(0, toolTotal - toolVisible);
  const mcpTotal = mcp.data?.servers.length ?? 0;
  const mcpConnected = connectedCount(mcp.data?.servers ?? []);

  return (
    <>
      {/* Overlay chỉ có ở mobile khi ngăn kéo đang mở; `<button>` để bấm ra ngoài
          đóng được bằng bàn phím. */}
      {open ? (
        <button
          type="button"
          aria-label={t("common.close")}
          className="fixed inset-0 z-20 bg-black/60 md:hidden"
          onClick={onClose}
        />
      ) : null}
      <aside
        data-testid="dashboard-sidebar"
        aria-label={t("hub.sidebar.title")}
        // Mobile: ngăn kéo ép mép phải. Desktop: đảo kính bo góc riêng — cùng
        // chất liệu với sidebar trái, sidebar chat và đảo nội dung.
        className={`glass fixed inset-y-0 right-0 z-30 flex w-72 flex-col border-l border-border transition-transform duration-200 md:static md:translate-x-0 md:rounded-2xl md:border ${open ? "translate-x-0" : "translate-x-full"}`}
      >
        {/* `h-14`: cùng chiều cao đảo TopNav nên kẻ ngang đầu ba đảo thẳng hàng. */}
        <div className="flex h-14 shrink-0 items-center justify-between gap-2 border-b border-border p-3">
          <h2 className="text-base font-bold">{t("hub.sidebar.title")}</h2>
          <button
            type="button"
            className="shrink-0 rounded-lg p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground md:hidden"
            onClick={onClose}
            aria-label={t("common.close")}
          >
            <XIcon className="size-5" aria-hidden="true" />
          </button>
        </div>
        {/* Các panel là nội dung tĩnh — cuộn bên trong sidebar, không đẩy đảo tràn màn hình. */}
        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
          <section
            aria-labelledby="hub-sidebar-tokens-title"
            data-testid="hub-sidebar-tokens"
            className="holo-panel rounded-md p-4"
          >
            <h3 id="hub-sidebar-tokens-title" className="holo-label text-xs font-semibold">
              {t("hub.sidebar.tokens")}
            </h3>
            <div className="mt-3 flex flex-col gap-4">
              {/* Token hôm nay so với ngân sách — vượt ngân sách thì gauge đỏ
                  VÀ nhãn chữ nói rõ, không chỉ qua màu. */}
              <div className="flex flex-col gap-1.5">
                <h4 className="text-xs text-ink-muted">{t("status.tokenUsage")}</h4>
                {used !== undefined ? (
                  <>
                    <p className="hub-num holo-glow text-2xl font-bold" data-testid="hub-holo-tokens">
                      {used.toLocaleString()}
                      <span className="text-sm font-medium text-ink-muted"> / {budget.toLocaleString()}</span>
                    </p>
                    <div className="h-1.5 overflow-hidden rounded-full bg-holo-faint">
                      <div
                        data-testid="hub-holo-tokens-gauge"
                        className={`h-full rounded-full ${over ? "bg-alert" : "bg-holo"}`}
                        style={{ width: `${percent}%` }}
                      />
                    </div>
                    <p className={`hub-num text-xs ${over ? "font-semibold text-alert" : "text-ink-muted"}`}>
                      {over ? t("status.budgetExceeded") : `${percent}%`}
                    </p>
                    {/* RBAC bật: ngân sách role là trần độc lập thứ hai (M21.7). */}
                    {status.data?.role !== undefined ? (
                      <p className="hub-num text-xs text-ink-muted" data-testid="hub-holo-role-tokens">
                        {t("status.roleTokenUsage", { role: status.data.role })}:{" "}
                        {(status.data.role_tokens_used ?? 0).toLocaleString()} /{" "}
                        {(status.data.role_daily_token_budget ?? 0).toLocaleString()}
                      </p>
                    ) : null}
                  </>
                ) : (
                  <p className="hub-num text-2xl font-bold" data-testid="hub-holo-tokens">
                    —
                  </p>
                )}
              </div>
              {/* Xu hướng 14 ngày — cùng dữ liệu với màn Status, bản rút gọn. */}
              <div className="flex flex-col gap-1.5">
                <h4 className="text-xs text-ink-muted">{t("hub.system.usage")}</h4>
                {!usage.isSuccess ? (
                  <p className="hub-num text-2xl font-bold" data-testid="hub-holo-trend-total">
                    —
                  </p>
                ) : !trendReady ? (
                  <p className="text-xs text-ink-muted">{t("usage.empty")}</p>
                ) : (
                  <>
                    <p className="hub-num holo-glow text-2xl font-bold" data-testid="hub-holo-trend-total">
                      {total.toLocaleString()}
                    </p>
                    <div
                      role="img"
                      aria-label={t("usage.summary", {
                        days: days.length,
                        total: total.toLocaleString(),
                        peak: peak !== null ? peak.total_tokens.toLocaleString() : "0",
                        peakDay: peak !== null ? formatDay(peak.day, lang) : "—",
                      })}
                      className="flex h-14 items-end gap-1"
                    >
                      {days.map((day) => {
                        const isPeak = peak !== null && day.day === peak.day && peak.total_tokens > 0;
                        return (
                          <div
                            key={day.day}
                            data-testid="holo-usage-bar"
                            data-day={day.day}
                            data-total={day.total_tokens}
                            title={`${formatDay(day.day, lang)} · ${day.total_tokens.toLocaleString()}`}
                            className={`min-h-[2px] flex-1 rounded-sm ${isPeak ? "bg-need" : "holo-bar"}`}
                            style={{
                              height: `${Math.max(2, Math.round((day.total_tokens / scale) * 100))}%`,
                            }}
                          />
                        );
                      })}
                    </div>
                    <p className="text-xs text-ink-muted">
                      {t("usage.range", {
                        from: formatDay(days[0].day, lang),
                        to: formatDay(days[days.length - 1].day, lang),
                      })}
                    </p>
                  </>
                )}
              </div>
            </div>
          </section>
          <section
            aria-labelledby="hub-activity-title"
            data-testid="hub-activity"
            className="holo-panel rounded-md p-4"
          >
            <div className="flex items-baseline justify-between gap-3">
              <h3 id="hub-activity-title" className="text-sm font-semibold">
                {t("hub.activity.title")}
              </h3>
              {entries.length > 0 ? (
                <Link
                  to="/audit"
                  className="text-xs text-ink-muted underline underline-offset-2 hover:text-ink"
                >
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
                  // Key ghép từ trường có sẵn trong audit, KHÔNG dùng chỉ số mảng:
                  // danh sách thay đổi khi có dòng mới, key theo index khiến React
                  // tái dùng sai node (giữ màu trạng thái sai).
                  <li
                    key={`${entry.ts}-${entry.session}-${entry.tool}-${entry.decided_by}`}
                    className="flex gap-2 text-xs"
                  >
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
          <section
            aria-labelledby="hub-system-title"
            data-testid="hub-system"
            className="holo-panel rounded-md p-4"
          >
            <h3 id="hub-system-title" className="text-sm font-semibold">
              {t("hub.system.title")}
            </h3>
            <dl className="mt-2 flex flex-col gap-1.5 text-xs">
              {/* Hạn mức hôm nay KHÔNG lặp ở đây: gauge token ở panel trên đã là
                  "một sự thật, một chỗ hiển thị" — hai nơi in cùng số chỉ tạo cơ
                  hội cho hai chỗ lệch nhau. Provider/endpoint cũng vậy: pill ở
                  sidebar trái là nơi hiển thị duy nhất. */}
              <Row
                label={t("status.version")}
                value={status.data?.version ?? "—"}
                mono
                testId="hub-system-version"
              />
              <Row label={t("status.model")} value={status.data?.model ?? "—"} mono />
              <Row
                label={t("status.uptime")}
                value={status.data ? formatUptime(status.data.uptime_seconds) : "—"}
                numeric
              />
              <Row
                label={t("status.channels")}
                value={status.data && status.data.channels.length > 0 ? status.data.channels.join(", ") : "—"}
                testId="hub-system-channels"
              />
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
            </dl>
          </section>
        </div>
      </aside>
    </>
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
