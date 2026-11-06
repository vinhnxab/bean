"use client";

import { Panel } from "@/components/ui/Page";
import { useSecrets } from "@/features/settings/queries";
import { SecretField } from "@/features/settings/SecretField";
import { useI18n } from "@/i18n";

/**
 * Mục **Tích hợp** — các khoá secret cho dịch vụ tích hợp (billing, marketing).
 * Không có endpoint kiểm tra kết nối cho nhóm này: chỉ quản lý `secrets.env`.
 */
export function IntegrationsPanel() {
  const { t } = useI18n();
  const { data: secrets } = useSecrets();

  const billingSlot = secrets?.slots.find((slot) => slot.name === "BILLING_API_KEY");
  const marketingSlot = secrets?.slots.find((slot) => slot.name === "MARKETING_API_KEY");

  return (
    <div className="max-w-3xl">
      <Panel>
        <h2 className="font-semibold">{t("settings.section.integrations")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("settings.section.integrationsDescription")}</p>
        <p className="mt-3 text-sm text-muted-foreground">{t("integrations.intro")}</p>

        {billingSlot ? (
          <>
            <h3 className="mt-5 mb-2 text-sm font-medium">{t("integrations.billing")}</h3>
            <SecretField slot={billingSlot} />
          </>
        ) : null}
        {marketingSlot ? (
          <>
            <h3 className="mt-5 mb-2 text-sm font-medium">{t("integrations.marketing")}</h3>
            <SecretField slot={marketingSlot} />
          </>
        ) : null}
      </Panel>
    </div>
  );
}
