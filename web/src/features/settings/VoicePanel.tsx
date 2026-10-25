import { CircleAlertIcon } from "lucide-react";

import { ApiRequestError } from "@/api/client";
import { ErrorState, LoadingState, Panel } from "@/components/ui/Page";
import { Switch } from "@/components/ui/switch";
import { useUpdateVoice, useVoiceStatus } from "@/features/chat/queries";
import { useI18n } from "@/i18n";

/**
 * Mục **Giọng nói** trong Cài đặt: bật/tắt nút mic push-to-talk (M27) lúc chạy,
 * không cần sửa `bean.toml` hay restart.
 *
 * Trạng thái đến từ `GET /api/voice` (`enabled` = provider đã dựng được **và**
 * chưa bị tắt qua UI). Switch chỉ tắt được khi `[voice]` trong `bean.toml` đã
 * khai báo đủ (`configured`); thiếu cấu hình thì UI khóa switch kèm hướng dẫn
 * thay vì để người dùng bấm rồi gặp lỗi server.
 */
export function VoicePanel() {
  const { t } = useI18n();
  const status = useVoiceStatus();
  const updateVoice = useUpdateVoice();
  const data = status.data;

  if (status.isPending) return <LoadingState label={t("common.loading")} />;
  if (status.isError || !data) return <ErrorState onRetry={() => void status.refetch()} />;

  const toggleError = updateVoice.isError
    ? updateVoice.error instanceof ApiRequestError && updateVoice.error.code !== "http_error"
      ? updateVoice.error.message
      : t("voice.toggleFailed")
    : null;

  return (
    <div className="max-w-3xl">
      <Panel>
        <h2 className="font-semibold">{t("voice.panelTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("voice.panelHint")}</p>

        <div className="mt-4 flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium">{t("voice.pushToTalk")}</span>
              <span className="text-xs text-muted-foreground">
                {data.configured
                  ? data.enabled
                    ? t("voice.state.on")
                    : t("voice.state.off")
                  : t("voice.state.notConfigured")}
              </span>
            </div>
            <dl className="mt-2 grid gap-1 text-sm">
              <div className="flex flex-wrap items-baseline justify-between gap-2">
                <dt className="text-muted-foreground">{t("voice.provider")}</dt>
                <dd className="text-right font-medium">{data.provider}</dd>
              </div>
              <div className="flex flex-wrap items-baseline justify-between gap-2">
                <dt className="text-muted-foreground">{t("voice.language")}</dt>
                <dd className="text-right font-mono text-xs">{data.language}</dd>
              </div>
            </dl>
          </div>
          <Switch
            checked={data.enabled}
            disabled={!data.configured || updateVoice.isPending}
            aria-label={t("voice.toggleAria")}
            onCheckedChange={(enabled) => updateVoice.mutate({ enabled })}
          />
        </div>

        {!data.configured ? (
          <p className="mt-3 text-sm text-muted-foreground">{t("voice.notConfiguredHint")}</p>
        ) : null}
        {data.configured && data.provider === "api" ? (
          <p className="mt-3 flex items-start gap-1.5 text-xs text-muted-foreground" role="note">
            <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
            {t("voice.privacyApi")}
          </p>
        ) : null}
        {toggleError ? (
          <p className="mt-3 flex items-center gap-1.5 text-sm font-medium text-destructive" role="alert">
            <CircleAlertIcon className="size-3.5 shrink-0" aria-hidden="true" />
            {toggleError}
          </p>
        ) : null}
      </Panel>
    </div>
  );
}
