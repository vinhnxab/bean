import { useMemo, useState } from "react";
import type { ToolDto, ToolListResponse } from "@/api/bindings";
import { Badge, StatusDot } from "@/components/ui/badge";
import { EmptyState, ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import { riskTone, useTools } from "@/features/tools/queries";
import { type Translate, useI18n } from "@/i18n";

/**
 * Màn **Tools** — "Bean đang có những khả năng nào".
 *
 * # Vì sao màn này đáng tồn tại
 *
 * Trước khi có `GET /api/tools`, câu trả lời duy nhất nằm trong `bean.toml`. Đây là
 * màn duy nhất cho thấy **description mà model đang đọc** — thứ quyết định nó chọn
 * tool nào, và là thứ dễ sai nhất khi thêm tool.
 *
 * # Vì sao có bộ lọc
 *
 * Lọc chỉ là tiện ích trên dữ liệu **đã được server lọc theo RBAC**. Client không
 * bao giờ là nơi phân quyền (xem `crates/bean-web/tests/hub_agents.rs`), nên lọc ký
 * tự ở đây an toàn: không tool nào bị ẩn vì quyền rồi hiện ra lại.
 */
export function ToolsPage() {
  const { t } = useI18n();
  const tools = useTools();
  const [query, setQuery] = useState("");
  const data = tools.data;

  const visible = useMemo(() => {
    const list = data?.tools ?? [];
    const needle = query.trim().toLowerCase();
    if (needle === "") return list;
    return list.filter((tool) => matches(tool, needle));
  }, [data, query]);

  return (
    <PageShell title={t("tools.title")} description={t("tools.description")}>
      {tools.isPending ? <LoadingState label={t("common.loading")} /> : null}
      {tools.isError ? <ErrorState onRetry={() => void tools.refetch()} /> : null}
      {data ? (
        <div className="space-y-4">
          <Summary data={data} t={t} />
          <div className="flex flex-wrap items-center gap-3">
            <label className="min-w-64 flex-1 text-sm">
              <span className="sr-only">{t("tools.search")}</span>
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t("tools.search")}
                className="field-input"
              />
            </label>
            <p className="text-xs text-muted-foreground">{t("tools.riskNote")}</p>
          </div>

          {visible.length === 0 ? (
            <EmptyState
              title={data.tools.length === 0 ? t("tools.empty.title") : t("tools.noMatch.title")}
              description={
                data.tools.length === 0 ? t("tools.empty.description") : t("tools.noMatch.description")
              }
            />
          ) : (
            <ul className="grid gap-3 lg:grid-cols-2">
              {visible.map((tool) => (
                <ToolCard key={tool.name} tool={tool} t={t} />
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </PageShell>
  );
}

/** Đếm nhanh theo mức rủi ro + nguồn, và nói thẳng RBAC có đang cắt danh sách không. */
function Summary({ data, t }: { data: ToolListResponse; t: Translate }) {
  const dangerous = data.tools.filter((tool) => tool.risk === "dangerous").length;
  const confirm = data.tools.filter((tool) => tool.risk === "confirm").length;
  const safe = data.tools.filter((tool) => tool.risk === "safe").length;
  const mcp = data.tools.filter((tool) => tool.source === "mcp").length;
  // `total` là tổng registry: hai số lệch nhau là **tín hiệu**, không phải lỗi.
  const hidden = data.rbac_enabled && data.total > data.tools.length;

  return (
    <Panel className="flex flex-wrap items-center gap-x-6 gap-y-3">
      <Metric label={t("tools.count.visible")} value={data.tools.length} total={data.total} />
      <Metric label={t("tools.count.mcp")} value={mcp} />
      <RiskPill risk="dangerous" count={dangerous} t={t} />
      <RiskPill risk="confirm" count={confirm} t={t} />
      <RiskPill risk="safe" count={safe} t={t} />
      {hidden ? (
        <p className="w-full text-xs text-muted-foreground">
          {t("tools.hiddenByRole", {
            visible: data.tools.length,
            total: data.total,
            role: data.viewer_role,
          })}
        </p>
      ) : null}
    </Panel>
  );
}

function Metric({ label, value, total }: { label: string; value: number; total?: number }) {
  return (
    <div>
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="hub-num mt-1 text-xl font-bold">
        {value.toLocaleString()}
        {total !== undefined && total !== value ? (
          <span className="text-sm font-medium text-muted-foreground"> / {total.toLocaleString()}</span>
        ) : null}
      </p>
    </div>
  );
}

function RiskPill({ risk, count, t }: { risk: ToolDto["risk"]; count: number; t: Translate }) {
  const tone = riskTone(risk);
  return (
    <Badge tone={tone}>
      <StatusDot tone={tone} />
      {t(`confirm.risk.${risk}`)}
      <span className="hub-num font-semibold">{count}</span>
    </Badge>
  );
}

function ToolCard({ tool, t }: { tool: ToolDto; t: Translate }) {
  const tone = riskTone(tool.risk);
  return (
    <Panel className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-mono text-sm font-semibold break-all">{tool.name}</h2>
        <Badge tone={tone}>
          <StatusDot tone={tone} />
          {t(`confirm.risk.${tool.risk}`)}
        </Badge>
        {tool.source === "mcp" && tool.mcp_server ? (
          <span className="font-mono text-xs text-muted-foreground">MCP · {tool.mcp_server}</span>
        ) : null}
      </div>
      <p className="text-sm text-muted-foreground">{tool.description}</p>
      <div className="flex flex-wrap items-center gap-1.5">
        {tool.untrusted ? (
          <span className="rounded-md border border-need/50 px-2 py-0.5 text-xs text-need">
            {t("tools.untrusted")}
          </span>
        ) : null}
        {tool.required_tags.map((tag) => (
          <span
            key={tag}
            className="rounded-md border border-rule px-2 py-0.5 font-mono text-xs text-muted-foreground"
          >
            {tag}
          </span>
        ))}
      </div>
    </Panel>
  );
}

/** Tìm theo tên, mô tả, tag hoặc tên MCP server — không phân biệt hoa thường. */
function matches(tool: ToolDto, needle: string): boolean {
  return (
    tool.name.toLowerCase().includes(needle) ||
    tool.description.toLowerCase().includes(needle) ||
    (tool.mcp_server?.toLowerCase().includes(needle) ?? false) ||
    tool.required_tags.some((tag) => tag.toLowerCase().includes(needle))
  );
}
