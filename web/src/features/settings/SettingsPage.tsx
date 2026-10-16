import { Navigate, useParams } from "react-router";

import { PageShell } from "@/components/ui/Page";
import { ProvidersPanel } from "@/features/settings/ProvidersPanel";
import { useI18n } from "@/i18n";

/**
 * Màn **Cài đặt** (phương án B): một trang hub, điều hướng giữa các section nằm
 * ở **sidebar phải** (`SettingsSidebar`) thay vì nav dọc trong trang.
 *
 * Section không tồn tại ⇒ rơi về `/settings/providers`: deep-link sai vẫn thấy
 * một màn có nghĩa thay vì trang trống.
 */
export function SettingsPage() {
  const { t } = useI18n();
  const { section } = useParams<{ section: string }>();

  if (section !== "providers") {
    return <Navigate to="/settings/providers" replace />;
  }

  return (
    <PageShell title={t("settings.title")} description={t("settings.description")}>
      <ProvidersPanel />
    </PageShell>
  );
}
