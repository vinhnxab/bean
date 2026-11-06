"use client";

import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Panel } from "@/components/ui/Page";
import { ConnectionTestButton } from "@/features/settings/ConnectionTestButton";
import { usePatchSetting, useSecrets, useSettings } from "@/features/settings/queries";
import { SecretField } from "@/features/settings/SecretField";
import { useI18n } from "@/i18n";

/**
 * Mục **Tìm kiếm web** (M7): provider Tavily/Brave/SearXNG + base URL (SearXNG tự host)
 * và ô secret API key (write-only). Nút kiểm tra kết nối gọi một request tìm tối
 * thiểu — cùng probe với `bean doctor --online`.
 */
export function SearchPanel() {
  const { t } = useI18n();
  const { data: settings } = useSettings();
  const { data: secrets } = useSecrets();
  const patchSetting = usePatchSetting();

  const providerRow = settings?.settings.find((row) => row.key === "tools.web_search.provider");
  const baseUrlRow = settings?.settings.find((row) => row.key === "tools.web_search.base_url");
  const searchSlot = secrets?.slots.find((slot) => slot.group === "search");

  const [providerValue, setProviderValue] = useState(providerRow?.value ?? "");
  const [baseValue, setBaseValue] = useState(baseUrlRow?.value ?? "");

  const handleProviderChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    setProviderValue(event.target.value);
    void patchSetting.mutate({
      key: "tools.web_search.provider",
      value: event.target.value,
    });
  };

  const handleBaseUrlChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    setBaseValue(event.target.value);
    void patchSetting.mutate({
      key: "tools.web_search.base_url",
      value: event.target.value,
    });
  };

  return (
    <div className="max-w-3xl">
      <Panel>
        <h2 className="font-semibold">{t("settings.section.search")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("settings.section.searchDescription")}</p>

        <form className="mt-5 grid gap-4 border-t border-border pt-4">
          <div className="grid gap-1.5">
            <label htmlFor="search-provider" className="text-sm font-medium">
              {t("search.provider")}
            </label>
            <Input
              id="search-provider"
              value={providerValue}
              onChange={handleProviderChange}
              placeholder={t("search.providerPlaceholder")}
            />
            <p className="mt-0.5 text-xs text-muted-foreground">{t("search.providerHint")}</p>
          </div>

          <div className="grid gap-1.5">
            <label htmlFor="search-base-url" className="text-sm font-medium">
              {t("search.baseUrl")}
            </label>
            <Input
              id="search-base-url"
              value={baseValue}
              onChange={handleBaseUrlChange}
              placeholder={t("search.baseUrlPlaceholder")}
            />
            <p className="mt-0.5 text-xs text-muted-foreground">{t("search.baseUrlHint")}</p>
          </div>
        </form>
      </Panel>

      <Panel className="mt-4">
        <h2 className="font-semibold">{t("search.keyPanelTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("search.keyPanelHint")}</p>
        {searchSlot ? (
          <SecretField slot={searchSlot} />
        ) : (
          <p className="text-sm text-muted-foreground">{t("search.noKeySlot")}</p>
        )}
        <div className="mt-4 flex gap-2">
          <ConnectionTestButton target="search" />
        </div>
      </Panel>
    </div>
  );
}
