import { CheckCircle2Icon, CircleAlertIcon } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ErrorState, LoadingState, Panel } from "@/components/ui/Page";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useLlmConfig, useUpdateLlmModel } from "@/features/settings/queries";
import { useI18n } from "@/i18n";

/**
 * Mục **Nhà cung cấp** trong Cài đặt (giai đoạn 1).
 *
 * # Ranh giới chỉnh sửa
 *
 * Chỉ **model** đổi được runtime — qua cùng đường với lệnh `/model` và chỉ trong
 * `allowed_models` server trả về (select không thể gửi giá trị ngoài danh sách).
 * Provider, base URL và API key chỉ đọc ở đây: đổi chúng phải rebuild HTTP client
 * kèm key mới, nên đổi trong `bean.toml` rồi khởi động lại Bean — UI nói rõ điều
 * đó thay vì hiện control chết. API key không bao giờ được hiển thị hay nhập tại
 * đây: chỉ có *tên biến môi trường* và cờ đã cấu hình hay chưa (mục 15.6).
 */
export function ProvidersPanel() {
  const { t } = useI18n();
  const config = useLlmConfig();
  const updateModel = useUpdateLlmModel();
  const data = config.data;
  // Model đang chọn trong form; chỉ khác `data.model` khi người dùng chưa bấm Lưu.
  // Sau khi Lưu, mutation cập nhật `data.model` đúng bằng lựa chọn này nên `dirty`
  // tự false và combobox khớp server — không cần state reset thủ công.
  const [model, setModel] = useState<string | null>(null);
  const current = model ?? data?.model ?? "";
  const dirty = data !== undefined && current !== data.model;

  if (config.isPending) return <LoadingState label={t("common.loading")} />;
  if (config.isError || !data) return <ErrorState onRetry={() => void config.refetch()} />;

  return (
    <div className="max-w-3xl">
      <Panel>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (!dirty || updateModel.isPending) return;
            updateModel.mutate({ model: current });
          }}
        >
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="font-semibold">{t("providers.model")}</h2>
              <p className="mt-1 text-sm text-muted-foreground">{t("providers.modelHint")}</p>
            </div>
            <div className="flex items-center gap-2">
              <Select
                value={current}
                onValueChange={(value) => {
                  // Chọn lại = bắt đầu một lần sửa mới: xoá thông báo của lần lưu trước.
                  updateModel.reset();
                  setModel(value);
                }}
              >
                <SelectTrigger
                  size="sm"
                  className="max-w-72"
                  aria-label={t("providers.model")}
                  data-slot="model-select"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {data.allowed_models.map((allowed) => (
                    <SelectItem key={allowed} value={allowed}>
                      {allowed}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Button type="submit" size="sm" disabled={!dirty || updateModel.isPending}>
                {updateModel.isPending ? t("common.loading") : t("common.save")}
              </Button>
            </div>
          </div>

          {/* Kết quả lưu: status (thành công) và alert (lỗi) tách vai trò cho
              trình đọc màn hình — một cái là thông báo, một cái là lỗi cần sửa. */}
          {updateModel.isSuccess ? (
            <p role="status" className="mt-3 flex items-center gap-2 text-sm text-live">
              <CheckCircle2Icon className="size-4 shrink-0" aria-hidden="true" />
              {t("providers.saved")}
            </p>
          ) : null}
          {updateModel.isError ? (
            <p role="alert" className="mt-3 flex items-center gap-2 text-sm text-destructive">
              <CircleAlertIcon className="size-4 shrink-0" aria-hidden="true" />
              {t("providers.saveError")}
            </p>
          ) : null}
        </form>
      </Panel>

      <Panel className="mt-4">
        <dl className="space-y-1.5 text-sm">
          <Row label={t("providers.provider")} value={data.provider} mono />
          <Row label={t("providers.baseUrl")} value={data.base_url ?? "—"} mono />
          <Row label={t("providers.maxTokens")} value={data.max_tokens.toLocaleString()} />
          <Row label={t("providers.apiKeyEnv")} value={data.api_key_env} mono />
        </dl>
        <p
          className={
            data.key_configured
              ? "mt-3 flex items-center gap-2 text-sm text-muted-foreground"
              : "mt-3 flex items-center gap-2 text-sm font-medium text-destructive"
          }
        >
          {data.key_configured ? (
            <CheckCircle2Icon className="size-4 shrink-0 text-live" aria-hidden="true" />
          ) : (
            <CircleAlertIcon className="size-4 shrink-0" aria-hidden="true" />
          )}
          {data.key_configured ? t("providers.keyConfigured") : t("providers.keyMissing")}
        </p>
        <p className="mt-4 border-t border-border pt-3 text-xs text-muted-foreground">
          {t("providers.readOnlyNote")}
        </p>
      </Panel>
    </div>
  );
}

function Row({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex flex-wrap items-baseline justify-between gap-2">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className={mono ? "text-right font-mono text-xs break-all" : "text-right font-medium"}>{value}</dd>
    </div>
  );
}
