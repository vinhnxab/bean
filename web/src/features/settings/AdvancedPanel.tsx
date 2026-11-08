"use client";

import { Panel } from "@/components/ui/Page";
import { LanAccessCard } from "@/features/settings/LanAccessCard";
import { useSettings } from "@/features/settings/queries";
import { useI18n } from "@/i18n";

/**
 * Mục **Nâng cao**: khoá cấu hình tầng B (bảo mật) — chỉ xem, sửa được qua
 * CLI tương tác (`bean config set <khoá> <giá trị>`). Một số yêu cầu khởi động
 * lại Bean để có hiệu lực.
 */
export function AdvancedPanel() {
  const { t } = useI18n();
  const { data: settings } = useSettings();

  const tierB = settings?.settings.filter((row) => row.tier === "B") ?? [];

  return (
    <Panel>
      <h2 className="font-semibold">{t("settings.section.advanced")}</h2>
      <p className="mt-1 text-sm text-muted-foreground">{t("settings.section.advancedDescription")}</p>

      <LanAccessCard />

      <div className="mt-4 overflow-x-auto border border-border rounded-lg">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-border">
              <th className="font-medium text-muted-foreground px-3 py-2">{t("advanced.column.key")}</th>
              <th className="font-medium text-muted-foreground px-3 py-2">{t("advanced.column.value")}</th>
              <th className="font-medium text-muted-foreground px-3 py-2">{t("advanced.column.source")}</th>
              <th className="font-medium text-muted-foreground px-3 py-2">{t("advanced.column.apply")}</th>
            </tr>
          </thead>
          <tbody>
            {tierB.map((row) => (
              <tr key={row.key} className="border-b border-border group">
                <td className="px-3 py-3 font-medium truncate">{row.key}</td>
                <td className="px-3 py-3 truncate">{row.value}</td>
                <td className="px-3 py-3 text-muted-foreground">{row.source}</td>
                <td className="px-3 py-3">
                  {row.apply === "restart" ? t("advanced.applyRestart") : t("advanced.applyHot")}
                </td>
              </tr>
            ))}
            {tierB.length === 0 && (
              <tr>
                <td colSpan={4} className="px-3 py-4 text-center text-muted-foreground">
                  {t("advanced.noTierB")}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <p className="mt-3 text-sm text-muted-foreground">{t("advanced.tierBHint")}</p>
      <p className="mt-1 text-xs text-muted-foreground truncate">{t("advanced.cliHint")}</p>
    </Panel>
  );
}
