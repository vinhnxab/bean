import { CheckCircle2Icon, CircleAlertIcon } from "lucide-react";
import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ErrorState, LoadingState, Panel } from "@/components/ui/Page";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useLlmConfig, useSwitchLlmProvider, useUpdateLlmModel } from "@/features/settings/queries";
import { useI18n } from "@/i18n";

/**
 * Mục **Nhà cung cấp** trong Cài đặt (giai đoạn 2: hot-swap provider).
 *
 * # Ranh giới chỉnh sửa
 *
 * Người dùng switch giữa các profile khai báo sẵn trong `[[llm.providers]]`
 * (`bean.toml`) — server tự dựng lại HTTP client và resolve API key từ biến môi
 * trường; key **không bao giờ** được nhập/hiển thị tại đây (mục 15.6). Thêm/xoá
 * profile vẫn là sửa `bean.toml` + restart: UI chỉ đọc danh sách server trả về.
 */
export function ProvidersPanel() {
  const { t } = useI18n();
  const config = useLlmConfig();
  const switchProvider = useSwitchLlmProvider();
  const updateModel = useUpdateLlmModel();
  const data = config.data;
  // Lựa chọn đang sửa trong form; `null` = theo snapshot server (chưa bấm gì).
  const [profile, setProfile] = useState<string | null>(null);
  const [model, setModel] = useState<string | null>(null);

  if (config.isPending) return <LoadingState label={t("common.loading")} />;
  if (config.isError || !data) return <ErrorState onRetry={() => void config.refetch()} />;

  const activeName = data.providers.find((entry) => entry.active)?.name ?? "default";
  const selectedName = profile ?? activeName;
  const selected = data.providers.find((entry) => entry.name === selectedName) ?? data.providers[0];
  // Profile đang active: combobox phản chiếu model server đang dùng (`/model` có thể
  // đã đổi khác model gốc của profile). Profile khác: bắt đầu từ model mặc định.
  const initialModel = selectedName === activeName ? data.model : selected.model;
  const currentModel = model ?? initialModel;
  const providerDirty = selectedName !== activeName;
  const modelDirty = currentModel !== (providerDirty ? selected.model : data.model);
  const dirty = providerDirty || modelDirty;
  const pending = switchProvider.isPending || updateModel.isPending;

  const save = (event: React.FormEvent) => {
    event.preventDefault();
    if (!dirty || pending) return;
    if (providerDirty) {
      // Model chỉ gửi kèm khi người dùng chủ ý chọn khác model mặc định của profile
      // (`null` = dùng model mặc định của profile — serde `default` map về None).
      switchProvider.mutate({
        profile: selectedName,
        model: modelDirty ? currentModel : null,
      });
    } else {
      updateModel.mutate({ model: currentModel });
    }
  };

  return (
    <div className="max-w-3xl">
      <Panel>
        <form onSubmit={save}>
          <h2 className="font-semibold">{t("providers.sectionTitle")}</h2>
          <p className="mt-1 text-sm text-muted-foreground">{t("providers.profilesHint")}</p>

          <div role="radiogroup" aria-label={t("providers.sectionTitle")} className="mt-4 grid gap-2">
            {data.providers.map((entry) => {
              const isSelected = entry.name === selectedName;
              return (
                <label
                  key={entry.name}
                  className={
                    isSelected
                      ? "flex cursor-pointer items-start gap-3 rounded-lg border border-brand bg-brand/5 p-3 transition-colors"
                      : "flex cursor-pointer items-start gap-3 rounded-lg border border-border p-3 transition-colors hover:border-brand/40"
                  }
                >
                  <input
                    type="radio"
                    name="llm-provider"
                    value={entry.name}
                    checked={isSelected}
                    onChange={() => {
                      // Bắt đầu một lần sửa mới: xoá thông báo của lần lưu trước,
                      // model quay về mặc định của profile vừa chọn.
                      switchProvider.reset();
                      updateModel.reset();
                      setProfile(entry.name);
                      setModel(null);
                    }}
                    className="mt-1 size-4 shrink-0 accent-brand"
                    aria-label={entry.name}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="flex flex-wrap items-center gap-2">
                      <span className="font-mono text-sm font-medium">{entry.name}</span>
                      {entry.active ? <Badge tone="working">{t("providers.activeBadge")}</Badge> : null}
                    </span>
                    <span className="mt-1 block truncate font-mono text-xs text-muted-foreground">
                      {entry.provider}
                      {entry.base_url ? ` · ${entry.base_url}` : ""}
                    </span>
                    <KeyStatus apiKeyEnv={entry.api_key_env} keyConfigured={entry.key_configured} />
                  </span>
                </label>
              );
            })}
          </div>

          <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-border pt-4">
            <div className="min-w-0">
              <p className="text-sm font-medium">{t("providers.model")}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">{t("providers.modelHint")}</p>
            </div>
            <div className="flex items-center gap-2">
              <Select
                value={currentModel}
                onValueChange={(value) => {
                  // Radix fire `""` khi đóng danh sách mà không chọn (Escape) —
                  // không phải lựa chọn thật, bỏ qua để không gửi `model: ""`.
                  if (!value) return;
                  switchProvider.reset();
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
                  {selected.allowed_models.map((allowed) => (
                    <SelectItem key={allowed} value={allowed}>
                      {allowed}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Button type="submit" size="sm" disabled={!dirty || pending}>
                {pending ? t("common.loading") : t("common.save")}
              </Button>
            </div>
          </div>

          {/* Kết quả lưu: status (thành công) và alert (lỗi) tách vai trò cho
              trình đọc màn hình — một cái là thông báo, một cái là lỗi cần sửa. */}
          {switchProvider.isSuccess ? (
            <p role="status" className="mt-3 flex items-center gap-2 text-sm text-live">
              <CheckCircle2Icon className="size-4 shrink-0" aria-hidden="true" />
              {t("providers.switched")}
            </p>
          ) : updateModel.isSuccess ? (
            <p role="status" className="mt-3 flex items-center gap-2 text-sm text-live">
              <CheckCircle2Icon className="size-4 shrink-0" aria-hidden="true" />
              {t("providers.saved")}
            </p>
          ) : null}
          {switchProvider.isError || updateModel.isError ? (
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
        <KeyStatus apiKeyEnv={data.api_key_env} keyConfigured={data.key_configured} large />
        <p className="mt-4 border-t border-border pt-3 text-xs text-muted-foreground">
          {t("providers.readOnlyNote")}
        </p>
      </Panel>
    </div>
  );
}

/** Trạng thái API key của một provider: không cần / đã có / thiếu trong môi trường. */
function KeyStatus({
  apiKeyEnv,
  keyConfigured,
  large = false,
}: {
  apiKeyEnv: string;
  keyConfigured: boolean;
  large?: boolean;
}) {
  const { t } = useI18n();
  const base = large ? "mt-3 text-sm" : "mt-1.5 text-xs";
  if (apiKeyEnv === "") {
    return (
      <span className={`${base} flex items-center gap-1.5 text-muted-foreground`}>
        <CheckCircle2Icon className="size-3.5 shrink-0 text-live" aria-hidden="true" />
        {t("providers.noKeyNeeded")}
      </span>
    );
  }
  if (keyConfigured) {
    return (
      <span className={`${base} flex items-center gap-1.5 text-muted-foreground`}>
        <CheckCircle2Icon className="size-3.5 shrink-0 text-live" aria-hidden="true" />
        {t("providers.keyConfigured")}
      </span>
    );
  }
  return (
    <span className={`${base} flex items-center gap-1.5 font-medium text-destructive`} role="alert">
      <CircleAlertIcon className="size-3.5 shrink-0" aria-hidden="true" />
      {t("providers.keyMissing")}
    </span>
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
