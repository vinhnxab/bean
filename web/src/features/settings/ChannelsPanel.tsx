"use client";

import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Panel } from "@/components/ui/Page";
import { Switch } from "@/components/ui/switch";
import { ConnectionTestButton } from "@/features/settings/ConnectionTestButton";
import { usePatchSetting, useSecrets, useSettings } from "@/features/settings/queries";
import { SecretField } from "@/features/settings/SecretField";
import { useI18n } from "@/i18n";

/**
 * Mục **Kênh** — Telegram (mục 13 AGENTS.md).
 *
 * Hiển thị công tắc bật/tắt + giới hạn tần suất (khoá A, sửa được từ UI) và
 * ô secret `TELEGRAM_BOT_TOKEN` (write-only, step-up ở server). Nút kiểm tra
 * kết nối gọi Telegram `getMe` — cùng probe với `bean doctor --online`.
 */
export function ChannelsPanel() {
  const { t } = useI18n();
  const { data: settings } = useSettings();
  const { data: secrets } = useSecrets();
  const patchSetting = usePatchSetting();

  const enabledRow = settings?.settings.find((row) => row.key === "telegram.enabled");
  const rateLimitRow = settings?.settings.find((row) => row.key === "telegram.rate_limit_per_minute");
  const tokenSlot = secrets?.slots.find((slot) => slot.name === "TELEGRAM_BOT_TOKEN");

  const enabledValue = enabledRow?.value === "true";
  const [rateValue, setRateValue] = useState(rateLimitRow?.value ?? "");

  const handleToggle = () => {
    void patchSetting.mutate({
      key: "telegram.enabled",
      value: String(!enabledValue),
    });
  };

  const handleRateChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    setRateValue(event.target.value);
    void patchSetting.mutate({
      key: "telegram.rate_limit_per_minute",
      value: event.target.value,
    });
  };

  return (
    <div className="max-w-3xl">
      <Panel>
        <h2 className="font-semibold">{t("settings.section.channels")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("settings.section.channelsDescription")}</p>

        <form className="mt-5 grid gap-4 border-t border-border pt-4">
          <div className="grid items-center gap-3">
            <label htmlFor="telegram-enabled" className="text-sm font-medium">
              {t("channels.enabled")}
            </label>
            <Switch checked={enabledValue} onCheckedChange={handleToggle} aria-checked={enabledValue} />
            <span className="text-sm text-muted-foreground">{t("channels.enabledHint")}</span>
          </div>

          <div className="grid gap-1.5">
            <label htmlFor="telegram-rate-limit" className="text-sm font-medium">
              {t("channels.rateLimit")}
            </label>
            <Input
              id="telegram-rate-limit"
              type="text"
              value={rateValue}
              onChange={handleRateChange}
              placeholder="20"
              disabled={!enabledValue}
            />
            <p className="mt-0.5 text-xs text-muted-foreground">{t("channels.rateLimitHint")}</p>
          </div>
        </form>
      </Panel>

      <Panel className="mt-4">
        <h2 className="font-semibold">{t("channels.tokenPanelTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("channels.tokenPanelHint")}</p>
        {tokenSlot ? (
          <SecretField slot={tokenSlot} />
        ) : (
          <p className="text-sm text-muted-foreground">{t("channels.noTokenSlot")}</p>
        )}
        <div className="mt-4 flex gap-2">
          <ConnectionTestButton target="telegram" disabled={!tokenSlot?.set} />
        </div>
      </Panel>
    </div>
  );
}
