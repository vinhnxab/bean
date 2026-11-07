import { Navigate, useParams } from "react-router";

import { PageShell } from "@/components/ui/Page";
import { AdvancedPanel } from "@/features/settings/AdvancedPanel";
import { ChannelsPanel } from "@/features/settings/ChannelsPanel";
import { IntegrationsPanel } from "@/features/settings/IntegrationsPanel";
import { PasswordPanel } from "@/features/settings/PasswordPanel";
import { ProvidersPanel } from "@/features/settings/ProvidersPanel";
import { useSettings } from "@/features/settings/queries";
import { SearchPanel } from "@/features/settings/SearchPanel";
import { VoicePanel } from "@/features/settings/VoicePanel";
import { useI18n } from "@/i18n";

/**
 * Khoá apply `restart` đã đổi nguồn so với mặc định (ví dụ sửa qua CLI) — Bean
 * cần khởi động lại để áp dụng. UI chỉ hiển thị cảnh báo, không tự khởi động
 * lại (cụm một instance, xem scripts/beanctl).
 */
function RestartBanner({ keys }: { keys: string[] }) {
  const { t } = useI18n();
  if (keys.length === 0) return null;
  return (
    <div className="mt-4 rounded-lg border border-amber-200 bg-amber-50/50 p-3 text-xs text-amber-800 dark:border-amber-900/30 dark:bg-amber-900/20 dark:text-amber-300">
      {t("settings.restartBanner", { keys: keys.join(", ") })}
    </div>
  );
}

const VALID_SECTIONS = [
  "providers",
  "voice",
  "password",
  "channels",
  "search",
  "integrations",
  "advanced",
] as const;

export function SettingsPage() {
  const { t } = useI18n();
  const { section } = useParams<{ section: string }>();
  const { data: settings } = useSettings();

  if (!VALID_SECTIONS.includes(section as (typeof VALID_SECTIONS)[number])) {
    return <Navigate to="/settings/providers" replace />;
  }

  const pendingRestart = (settings?.settings ?? []).filter(
    (row) => row.apply === "restart" && row.source !== "default",
  );

  return (
    <PageShell title={t("settings.title")} description={t("settings.description")} className="max-w-none">
      {pendingRestart.length > 0 ? <RestartBanner keys={pendingRestart.map((row) => row.key)} /> : null}
      {section === "voice" ? (
        <VoicePanel />
      ) : section === "password" ? (
        <PasswordPanel />
      ) : section === "channels" ? (
        <ChannelsPanel />
      ) : section === "search" ? (
        <SearchPanel />
      ) : section === "integrations" ? (
        <IntegrationsPanel />
      ) : section === "advanced" ? (
        <AdvancedPanel />
      ) : (
        <ProvidersPanel />
      )}
    </PageShell>
  );
}
