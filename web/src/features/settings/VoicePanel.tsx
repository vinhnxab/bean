import { CircleAlertIcon } from "lucide-react";
import { useState } from "react";

import type { VoiceStatusResponse, VoiceUpdateRequest } from "@/api/bindings";
import { ApiRequestError } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ErrorState, LoadingState, Panel } from "@/components/ui/Page";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { useUpdateVoice, useVoiceStatus } from "@/features/chat/queries";
import { useI18n } from "@/i18n";

/**
 * Mục **Giọng nói** trong Cài đặt: bật/tắt nút mic push-to-talk **và** cấu hình
 * provider (local / ElevenLabs API), ngôn ngữ, API key — tất cả đổi lúc chạy qua
 * `PATCH /api/voice`, không cần sửa `bean.toml` hay khởi động lại Bean.
 *
 * Trạng thái đến từ `GET /api/voice` (`configured` = provider đã dựng được). Khi
 * `configured = false` UI vẫn hiện form — hoàn thiện provider/key tại đây là cách
 * cấu hình chính, không phải chỉ hiển thị chỉ dẫn.
 */
export function VoicePanel() {
  const { t } = useI18n();
  const status = useVoiceStatus();
  const updateVoice = useUpdateVoice();
  const data = status.data;

  if (status.isPending) return <LoadingState label={t("common.loading")} />;
  if (status.isError || !data) return <ErrorState onRetry={() => void status.refetch()} />;

  return (
    <div className="max-w-3xl">
      <Panel>
        <h2 className="font-semibold">{t("voice.panelTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("voice.panelHint")}</p>
        <VoiceForm
          // Mount lại (reset bản nháp về snapshot) mỗi khi cấu hình hiệu lực trên
          // server đổi — sau PATCH thành công. KHÔNG gồm `enabled`: toggle không
          // đổi cấu hình, và remount giữa chừng sẽ xoá nội dung người dùng đang gõ.
          key={`${data.provider}|${data.language}|${data.api_key_set}`}
          data={data}
          // Mutation sống ở cha: form remount không được reset trạng thái
          // thành công/lỗi của lần PATCH vừa rồi (thông báo "Đã lưu" phải còn).
          updateVoice={updateVoice}
        />
      </Panel>
    </div>
  );
}

/**
 * Form cấu hình voice + công tắc bật/tắt.
 *
 * # Một PATCH = toàn bộ trạng thái đang hiển thị
 *
 * Công tắc và nút Lưu đều gửi **cả** form (provider/ngôn ngữ/key nếu có dán) kèm
 * giá trị `enabled` mới. Nhờ vậy không có tình huống "form đang dirty mà bấm công
 * tắc làm mất nội dung vừa gõ": mọi thay đổi nhìn thấy trên màn hình luôn là thứ
 * được persist. Server chỉ dựng lại provider khi cấu hình đổi — toggle thuần túy
 * không tái tạo subprocess/API client.
 */
function VoiceForm({
  data,
  updateVoice,
}: {
  data: VoiceStatusResponse;
  updateVoice: ReturnType<typeof useUpdateVoice>;
}) {
  const { t } = useI18n();

  // Bản nháp form, khởi tạo MỘT LẦN từ snapshot server (component được mount lại
  // bằng `key` khi snapshot đổi — xem `VoicePanel`). `apiKey` là ghi một chiều:
  // luôn rỗng lúc mount — server không bao giờ trả key về, để trống khi gửi là
  // "giữ nguyên key hiện tại".
  const [provider, setProvider] = useState(data.provider === "api" ? "api" : "local");
  const [language, setLanguage] = useState(data.language);
  const [apiKey, setApiKey] = useState("");

  const pending = updateVoice.isPending;
  const languageTrimmed = language.trim();
  const dirty = provider !== data.provider || languageTrimmed !== data.language || apiKey.trim() !== "";
  // Chế độ `api` phải có key từ đâu đó trước khi lưu: đã lưu trên server, env, hoặc
  // vừa dán vào form. Chặn ở client để tránh bắt người dùng gặp 422 tránh được.
  const canSubmit = provider === "local" || data.api_key_set || apiKey.trim() !== "";

  const request = (enabled: boolean): VoiceUpdateRequest => ({
    enabled,
    provider,
    language: languageTrimmed,
    ...(provider === "api" && apiKey.trim() !== "" ? { api_key: apiKey.trim() } : {}),
  });

  const save = (event: React.FormEvent) => {
    event.preventDefault();
    if (pending || !dirty || !canSubmit) return;
    // Chưa cấu hình xong thì "Lưu" đồng nghĩa "bật" — người dùng mở form này vì
    // muốn dùng voice; đã cấu hình thì giữ nguyên trạng thái bật/tắt hiện tại.
    updateVoice.mutate(request(data.configured ? data.enabled : true));
  };

  const saveError = updateVoice.isError
    ? updateVoice.error instanceof ApiRequestError && updateVoice.error.code !== "http_error"
      ? updateVoice.error.message
      : t("voice.updateFailed")
    : null;

  const stateText = data.configured
    ? data.enabled
      ? t("voice.state.on")
      : t("voice.state.off")
    : t("voice.state.notConfigured");

  return (
    <>
      <div className="mt-4 flex items-center justify-between gap-4">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">{t("voice.pushToTalk")}</span>
          <span className="text-xs text-muted-foreground">{stateText}</span>
        </div>
        <Switch
          checked={data.enabled}
          disabled={!data.configured || !canSubmit || pending}
          aria-label={t("voice.toggleAria")}
          onCheckedChange={(enabled) => updateVoice.mutate(request(enabled))}
        />
      </div>

      <form className="mt-5 grid gap-4 border-t border-border pt-4" onSubmit={save}>
        <div className="grid gap-1.5">
          <label htmlFor="voice-provider" className="text-sm font-medium">
            {t("voice.provider")}
          </label>
          <Select value={provider} onValueChange={setProvider}>
            <SelectTrigger id="voice-provider" className="w-full sm:w-72">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="local">{t("voice.providerLocal")}</SelectItem>
              <SelectItem value="api">{t("voice.providerApi")}</SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">{t("voice.providerHint")}</p>
        </div>

        <div className="grid gap-1.5">
          <label htmlFor="voice-language" className="text-sm font-medium">
            {t("voice.language")}
          </label>
          <Input
            id="voice-language"
            value={language}
            onChange={(event) => setLanguage(event.target.value)}
            placeholder={t("voice.languagePlaceholder")}
            autoComplete="off"
            spellCheck={false}
            className="w-full sm:w-72"
          />
          <p className="text-xs text-muted-foreground">{t("voice.languageHint")}</p>
        </div>

        {provider === "api" ? (
          <div className="grid gap-1.5">
            <label htmlFor="voice-api-key" className="text-sm font-medium">
              {t("voice.apiKey")}
            </label>
            <Input
              id="voice-api-key"
              type="password"
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder={data.api_key_set ? t("voice.apiKeyStored") : t("voice.apiKeyPastePlaceholder")}
              autoComplete="off"
              spellCheck={false}
              className="w-full sm:w-72"
            />
            {data.api_key_set ? null : (
              <p className="text-xs text-muted-foreground">{t("voice.apiKeyRequiredHint")}</p>
            )}
          </div>
        ) : null}

        {provider === "api" ? (
          <p className="flex items-start gap-1.5 text-xs text-muted-foreground" role="note">
            <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
            {t("voice.privacyApi")}
          </p>
        ) : null}

        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" disabled={!dirty || !canSubmit || pending}>
            {data.configured ? t("voice.save") : t("voice.saveAndEnable")}
          </Button>
          {updateVoice.isSuccess ? (
            <span role="status" className="text-xs text-muted-foreground">
              {t("voice.saved")}
            </span>
          ) : null}
        </div>

        {saveError ? (
          <p className="flex items-center gap-1.5 text-sm font-medium text-destructive" role="alert">
            <CircleAlertIcon className="size-3.5 shrink-0" aria-hidden="true" />
            {saveError}
          </p>
        ) : null}
      </form>

      {!data.configured ? (
        <p className="mt-3 text-sm text-muted-foreground">{t("voice.notConfiguredHint")}</p>
      ) : null}
    </>
  );
}
