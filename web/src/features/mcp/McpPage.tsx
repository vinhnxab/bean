import type { McpServerDto } from "@/api/bindings";
import { Badge, StatusDot } from "@/components/ui/badge";
import { EmptyState, ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import { useMcpServers } from "@/features/mcp/queries";
import { type Translate, useI18n } from "@/i18n";

/**
 * Màn **MCP** — "Bean đang nói chuyện được với server nào".
 *
 * # `connected` nghĩa là gì — và không nghĩa là gì
 *
 * Server được đánh dấu đã kết nối khi tool của nó **đang nằm trong registry**, tức
 * initialize + discovery đã thành công lúc khởi động. Đây không phải health check
 * sống/chết: một server treo giữa chừng vẫn hiện "đã kết nối" cho tới khi Bean khởi
 * động lại. Nói rõ điều này ở dòng hint, vì người đọc sẽ mặc định hiểu ngược lại.
 *
 * API cũng **không** trả `env` của server — màn này vì thế không có cách nào lỡ in
 * ra credential, kể cả khi ai đó thêm tính năng "xem cấu hình".
 */
export function McpPage() {
  const { t } = useI18n();
  const servers = useMcpServers();
  const data = servers.data;

  return (
    <PageShell title={t("mcp.title")} description={t("mcp.description")}>
      {servers.isPending ? <LoadingState label={t("common.loading")} /> : null}
      {servers.isError ? <ErrorState onRetry={() => void servers.refetch()} /> : null}
      {data ? (
        data.servers.length === 0 ? (
          <EmptyState title={t("mcp.empty.title")} description={t("mcp.empty.description")} />
        ) : (
          <>
            <p className="mb-4 text-xs text-muted-foreground">{t("mcp.hint")}</p>
            <ul className="grid gap-3 lg:grid-cols-2">
              {data.servers.map((server) => (
                <ServerCard key={server.name} server={server} t={t} />
              ))}
            </ul>
          </>
        )
      ) : null}
    </PageShell>
  );
}

function ServerCard({ server, t }: { server: McpServerDto; t: Translate }) {
  const tone = server.connected ? "working" : "idle";
  return (
    <Panel className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-mono text-sm font-semibold">{server.name}</h2>
        <Badge tone={tone}>
          <StatusDot tone={tone} />
          {server.connected ? t("mcp.connected") : t("mcp.disconnected")}
        </Badge>
        <span className="hub-num text-xs text-muted-foreground">
          {t("mcp.toolCount", { count: server.tool_count })}
        </span>
      </div>

      <p className="font-mono text-xs break-all text-muted-foreground">
        {[server.command, ...server.args].join(" ")}
      </p>

      <dl className="mt-1 space-y-1 text-xs">
        <Row label={t("mcp.trust")} value={server.trusted ? t("mcp.trusted") : t("mcp.confirmEach")} />
        <Row
          label={t("mcp.timeout")}
          value={
            server.call_timeout_seconds === null
              ? t("mcp.timeoutDefault")
              : t("mcp.timeoutSeconds", { seconds: server.call_timeout_seconds })
          }
        />
        {server.required_tags.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1.5 pt-1">
            {server.required_tags.map((tag) => (
              <span
                key={tag}
                className="rounded-md border border-rule px-2 py-0.5 font-mono text-xs text-muted-foreground"
              >
                {tag}
              </span>
            ))}
          </div>
        ) : null}
      </dl>
    </Panel>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="text-right font-medium">{value}</dd>
    </div>
  );
}
