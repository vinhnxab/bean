import { PageShell, Panel } from "@/components/ui/Page";
import { useI18n } from "@/i18n";
import type { MessageKey } from "@/i18n/vi";

/**
 * Các lệnh slash lõi xử lý ngay, không tốn lượt LLM (đặc tả mục 10). Thêm lệnh
 * mới ở lõi thì thêm một dòng ở đây — bảng này là tài liệu tham chiếu, không
 * phải nơi điều khiển hành vi.
 */
const SLASH_COMMANDS: { cmd: string; descKey: MessageKey }[] = [
  { cmd: "/new", descKey: "docs.cmd.new" },
  { cmd: "/stop", descKey: "docs.cmd.stop" },
  { cmd: "/model", descKey: "docs.cmd.model" },
  { cmd: "/skills", descKey: "docs.cmd.skills" },
  { cmd: "/memory", descKey: "docs.cmd.memory" },
  { cmd: "/tasks", descKey: "docs.cmd.tasks" },
  { cmd: "/approve <id>", descKey: "docs.cmd.approve" },
  { cmd: "/reject <id>", descKey: "docs.cmd.reject" },
];

/** Mẹo dùng màn chat — những thứ không nhìn thấy được từ giao diện. */
const TIPS: { tipKey: MessageKey }[] = [
  { tipKey: "docs.tip.enter" },
  { tipKey: "docs.tip.stop" },
  { tipKey: "docs.tip.confirm" },
  { tipKey: "docs.tip.memory" },
];

/**
 * Màn **Tài liệu** — tham chiếu tĩnh về cách điều khiển Bean: lệnh slash, mẹo
 * dùng màn chat. Nội dung ít đổi và không phụ thuộc API nên trang không gọi
 * query nào; hai cột gộp xuống một cột trên màn hẹp.
 */
export function DocsPage() {
  const { t } = useI18n();

  return (
    <PageShell title={t("docs.title")} description={t("docs.description")}>
      <div className="grid gap-4 lg:grid-cols-2">
        <Panel>
          <h2 className="text-sm font-semibold">{t("docs.slashTitle")}</h2>
          <ul className="mt-3 space-y-2">
            {SLASH_COMMANDS.map(({ cmd, descKey }) => (
              <li
                key={cmd}
                className="flex flex-col gap-0.5 rounded-lg border border-border/60 px-3 py-2 sm:flex-row sm:items-baseline sm:gap-3"
              >
                <code className="shrink-0 font-mono text-sm font-semibold text-brand">{cmd}</code>
                <span className="text-xs text-muted-foreground">{t(descKey)}</span>
              </li>
            ))}
          </ul>
        </Panel>
        <Panel>
          <h2 className="text-sm font-semibold">{t("docs.tipsTitle")}</h2>
          <ul className="mt-3 space-y-2">
            {TIPS.map(({ tipKey }) => (
              <li
                key={tipKey}
                className="rounded-lg border border-border/60 px-3 py-2 text-xs text-muted-foreground"
              >
                {t(tipKey)}
              </li>
            ))}
          </ul>
        </Panel>
      </div>
    </PageShell>
  );
}
