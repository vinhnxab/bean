import { CheckCircle2Icon, CircleAlertIcon, PencilIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { useState } from "react";
import type { LlmProviderInfo, LlmProviderUpdateRequest } from "@/api/bindings";
import { ApiRequestError } from "@/api/client";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ErrorState, LoadingState, Panel } from "@/components/ui/Page";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  useCreateLlmProvider,
  useDeleteLlmProvider,
  useLlmConfig,
  useSwitchLlmProvider,
  useUpdateLlmModel,
  useUpdateLlmProvider,
} from "@/features/settings/queries";
import { useI18n } from "@/i18n";

/** Đích của form thêm/sửa profile; `edit` giữ nguyên profile gốc để điền sẵn. */
type FormTarget = { mode: "create" } | { mode: "edit"; profile: LlmProviderInfo };

/**
 * Thông điệp lỗi cho người dùng: server trả message chi tiết theo từng trường
 * (422 `profile_invalid` v.v.) — ưu tiên hiển thị nguyên văn; `http_error` của
 * lớp transport (mạng, body lạ) thì fallback câu hướng dẫn chung.
 */
function mutationErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof ApiRequestError && error.code !== "http_error") {
    return error.message;
  }
  return fallback;
}

/**
 * Mục **Nhà cung cấp** trong Cài đặt (giai đoạn 2 + 3: hot-swap + CRUD profile).
 *
 * # Ranh giới chỉnh sửa
 *
 * Switch giữa các profile và thêm/sửa/xoá profile đều chạy ngay tại đây: CRUD ghi
 * thẳng `[[llm.providers]]` của `bean.toml` phía server (persist trước, áp dụng
 * runtime sau). API key là ghi **một chiều**: có thể dán key mới để lưu trong
 * `bean.toml`, nhưng giá trị không bao giờ được trả về UI — chỉ có hint 4 ký tự
 * cuối (`api_key_hint`). Máy chủ chạy không biết đường `bean.toml` (CLI/test) sẽ
 * trả 503 `config_readonly`.
 */
export function ProvidersPanel() {
  const { t } = useI18n();
  const config = useLlmConfig();
  const switchProvider = useSwitchLlmProvider();
  const updateModel = useUpdateLlmModel();
  const createProvider = useCreateLlmProvider();
  const updateProvider = useUpdateLlmProvider();
  const deleteProvider = useDeleteLlmProvider();
  const data = config.data;
  // Lựa chọn đang sửa trong form; `null` = theo snapshot server (chưa bấm gì).
  const [profile, setProfile] = useState<string | null>(null);
  const [model, setModel] = useState<string | null>(null);
  // CRUD (giai đoạn 3): form thêm/sửa + confirm xoá.
  const [formTarget, setFormTarget] = useState<FormTarget | null>(null);
  const [deleting, setDeleting] = useState<LlmProviderInfo | null>(null);

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
  const pending =
    switchProvider.isPending ||
    updateModel.isPending ||
    createProvider.isPending ||
    updateProvider.isPending ||
    deleteProvider.isPending;

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
          <div className="flex flex-wrap items-start justify-between gap-2">
            <div className="min-w-0">
              <h2 className="font-semibold">{t("providers.sectionTitle")}</h2>
              <p className="mt-1 text-sm text-muted-foreground">{t("providers.profilesHint")}</p>
            </div>
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="gap-1.5"
              data-slot="add-profile"
              onClick={() => {
                // Bắt đầu một phiên làm việc CRUD mới: xoá trạng thái của lần trước.
                createProvider.reset();
                updateProvider.reset();
                deleteProvider.reset();
                setFormTarget({ mode: "create" });
              }}
            >
              <PlusIcon className="size-4" aria-hidden="true" />
              {t("providers.addProfile")}
            </Button>
          </div>

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
                    <KeyStatus
                      apiKeyEnv={entry.api_key_env}
                      keyConfigured={entry.key_configured}
                      hint={entry.api_key_hint}
                    />
                  </span>
                  {entry.name === "default" ? null : (
                    <span className="flex shrink-0 items-center gap-1">
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="size-8"
                        aria-label={t("providers.editProfile")}
                        onClick={(event) => {
                          // Nút nằm trong <label> của radio: chặn để không chọn nhầm profile.
                          event.preventDefault();
                          switchProvider.reset();
                          updateModel.reset();
                          createProvider.reset();
                          updateProvider.reset();
                          deleteProvider.reset();
                          setFormTarget({ mode: "edit", profile: entry });
                        }}
                      >
                        <PencilIcon className="size-4" aria-hidden="true" />
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="size-8 text-destructive hover:text-destructive"
                        aria-label={t("providers.deleteProfile")}
                        onClick={(event) => {
                          event.preventDefault();
                          switchProvider.reset();
                          updateModel.reset();
                          createProvider.reset();
                          updateProvider.reset();
                          deleteProvider.reset();
                          setDeleting(entry);
                        }}
                      >
                        <Trash2Icon className="size-4" aria-hidden="true" />
                      </Button>
                    </span>
                  )}
                </label>
              );
            })}
          </div>
          <p className="mt-2 text-xs text-muted-foreground">{t("providers.defaultReadonlyHint")}</p>

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
          {createProvider.isSuccess ? (
            <p role="status" className="mt-3 flex items-center gap-2 text-sm text-live">
              <CheckCircle2Icon className="size-4 shrink-0" aria-hidden="true" />
              {t("providers.created")}
            </p>
          ) : updateProvider.isSuccess ? (
            <p role="status" className="mt-3 flex items-center gap-2 text-sm text-live">
              <CheckCircle2Icon className="size-4 shrink-0" aria-hidden="true" />
              {t("providers.updated")}
            </p>
          ) : deleteProvider.isSuccess ? (
            <p role="status" className="mt-3 flex items-center gap-2 text-sm text-live">
              <CheckCircle2Icon className="size-4 shrink-0" aria-hidden="true" />
              {t("providers.deleted")}
            </p>
          ) : null}
          {switchProvider.isError || updateModel.isError ? (
            <p role="alert" className="mt-3 flex items-center gap-2 text-sm text-destructive">
              <CircleAlertIcon className="size-4 shrink-0" aria-hidden="true" />
              {t("providers.saveError")}
            </p>
          ) : null}
          {deleteProvider.isError ? (
            <p role="alert" className="mt-3 flex items-center gap-2 text-sm text-destructive">
              <CircleAlertIcon className="size-4 shrink-0" aria-hidden="true" />
              {mutationErrorMessage(deleteProvider.error, t("providers.crudError"))}
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

      {/* Form thêm/sửa profile. `key` theo đích để state form luôn khởi tạo mới
          khi đổi giữa create/edit hoặc đổi profile đang sửa. */}
      {formTarget ? (
        <ProviderFormDialog
          key={formTarget.mode === "edit" ? `edit:${formTarget.profile.name}` : "create"}
          target={formTarget}
          create={createProvider}
          update={updateProvider}
          onClose={() => setFormTarget(null)}
        />
      ) : null}

      <ConfirmDialog
        open={deleting !== null}
        title={t("providers.confirmDeleteTitle")}
        message={t("providers.confirmDeleteMessage")}
        confirmLabel={t("common.delete")}
        cancelLabel={t("common.cancel")}
        danger
        pending={deleteProvider.isPending}
        onConfirm={() => {
          if (!deleting) return;
          deleteProvider.mutate(deleting.name, { onSuccess: () => setDeleting(null) });
        }}
        onCancel={() => setDeleting(null)}
      />
    </div>
  );
}

/**
 * Trạng thái API key của một provider: không cần / đã lưu trong file / đã có trong
 * môi trường / thiếu. `hint` là 4 ký tự cuối của key đã lưu trực tiếp (không bao
 * giờ là giá trị nguyên vẹn).
 */
function KeyStatus({
  apiKeyEnv,
  keyConfigured,
  hint,
  large = false,
}: {
  apiKeyEnv: string;
  keyConfigured: boolean;
  hint?: string | null;
  large?: boolean;
}) {
  const { t } = useI18n();
  const base = large ? "mt-3 text-sm" : "mt-1.5 text-xs";
  if (hint) {
    return (
      <span className={`${base} flex items-center gap-1.5 text-muted-foreground`}>
        <CheckCircle2Icon className="size-3.5 shrink-0 text-live" aria-hidden="true" />
        {t("providers.keyStored", { hint })}
      </span>
    );
  }
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

/**
 * Form thêm/sửa profile (giai đoạn 3). State khởi tạo từ `target` mỗi lần mount
 * (component unmount khi đóng), mutations nhận qua props để trạng thái
 * success/error sống sót sau khi dialog đóng và hiển thị trên panel.
 */
function ProviderFormDialog({
  target,
  create,
  update,
  onClose,
}: {
  target: FormTarget;
  create: ReturnType<typeof useCreateLlmProvider>;
  update: ReturnType<typeof useUpdateLlmProvider>;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const editing = target.mode === "edit" ? target.profile : null;
  const [name, setName] = useState(editing?.name ?? "");
  const [providerKind, setProviderKind] = useState(editing?.provider ?? "anthropic");
  const [model, setModel] = useState(editing?.model ?? "");
  const [allowedModels, setAllowedModels] = useState(editing?.allowed_models.join(", ") ?? "");
  const [apiKeyEnv, setApiKeyEnv] = useState(editing?.api_key_env ?? "");
  // API key dán trực tiếp (ghi một chiều): trống = giữ nguyên hiện trạng; chỉ có
  // giá trị khi người dùng chủ ý dán. `clearKey` tách riêng ý định xoá key đã lưu
  // (submit gửi `""`) để xoá-by-form-trống không bao giờ xảy ra.
  const [apiKey, setApiKey] = useState("");
  const [clearKey, setClearKey] = useState(false);
  const storedHint = editing?.api_key_hint ?? null;
  const [baseUrl, setBaseUrl] = useState(editing?.base_url ?? "");
  const [maxTokens, setMaxTokens] = useState(editing ? String(editing.max_tokens) : "");
  // Message lỗi của lần submit gần nhất; null = chưa lỗi. State nội bộ (không dựa
  // vào mutation.error) để dialog mở lại luôn sạch, còn status success vẫn sống
  // trên panel qua mutation như comment ở đầu component.
  const [submitError, setSubmitError] = useState<string | null>(null);

  const pending = create.isPending || update.isPending;

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (pending) return;
    const trimmedModel = model.trim();
    if (trimmedModel === "") return;
    setSubmitError(null);
    const pastedKey = apiKey.trim();
    const apiKeyUpdate = editing
      ? clearKey
        ? "" // người dùng bấm "Xoá key đã lưu" ⇒ Clear
        : pastedKey === ""
          ? null // form để trống ⇒ Keep (không đụng key đã lưu)
          : pastedKey // dán key mới ⇒ Set
      : pastedKey === ""
        ? null // tạo mới không dán key ⇒ resolve từ env
        : pastedKey;
    const request: LlmProviderUpdateRequest = {
      provider: providerKind,
      model: trimmedModel,
      allowed_models: allowedModels
        .split(",")
        .map((entry) => entry.trim())
        .filter((entry) => entry !== ""),
      api_key_env: apiKeyEnv.trim(),
      api_key: apiKeyUpdate,
      base_url: baseUrl.trim() === "" ? null : baseUrl.trim(),
      max_tokens: maxTokens.trim() === "" ? null : Number(maxTokens.trim()),
    };
    const onError = (error: Error) => {
      setSubmitError(mutationErrorMessage(error, t("providers.crudError")));
    };
    if (editing) {
      update.mutate({ name: editing.name, request }, { onSuccess: onClose, onError });
    } else {
      create.mutate({ name: name.trim(), ...request }, { onSuccess: onClose, onError });
    }
  };

  return (
    <Dialog
      open
      onOpenChange={(next) => {
        if (!next && !pending) onClose();
      }}
    >
      <DialogContent showClose={!pending}>
        <DialogHeader>
          <DialogTitle>{editing ? t("providers.editProfile") : t("providers.addProfile")}</DialogTitle>
          <DialogDescription>{editing ? editing.name : t("providers.apiKeyEnvHint")}</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          {editing ? null : (
            <div className="grid gap-1.5">
              <label htmlFor="provider-name" className="text-sm font-medium">
                {t("providers.name")}
              </label>
              <Input
                id="provider-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                required
                autoFocus
                maxLength={64}
                pattern="[a-z0-9][a-z0-9-]*"
                title={t("providers.nameHint")}
                data-slot="provider-name"
                className="font-mono"
              />
              <span className="text-xs text-muted-foreground">{t("providers.nameHint")}</span>
            </div>
          )}

          <div className="grid gap-1.5">
            <span className="text-sm font-medium">{t("providers.providerKind")}</span>
            <Select
              value={providerKind}
              onValueChange={(value) => {
                // Radix fire "" khi đóng danh sách mà không chọn (Escape) — bỏ qua.
                if (!value) return;
                setProviderKind(value);
              }}
            >
              <SelectTrigger data-slot="provider-kind" aria-label={t("providers.providerKind")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="anthropic">{t("providers.providerAnthropic")}</SelectItem>
                <SelectItem value="openai_compat">{t("providers.providerOpenaiCompat")}</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="grid gap-1.5">
            <label htmlFor="provider-model" className="text-sm font-medium">
              {t("providers.model")}
            </label>
            <Input
              id="provider-model"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              required
              data-slot="provider-model"
              className="font-mono"
            />
          </div>

          <div className="grid gap-1.5">
            <label htmlFor="provider-allowed-models" className="text-sm font-medium">
              {t("providers.allowedModels")}
            </label>
            <Input
              id="provider-allowed-models"
              value={allowedModels}
              onChange={(event) => setAllowedModels(event.target.value)}
              data-slot="provider-allowed-models"
              className="font-mono"
            />
            <span className="text-xs text-muted-foreground">{t("providers.allowedModelsHint")}</span>
          </div>

          <div className="grid gap-1.5">
            <label htmlFor="provider-api-key-env" className="text-sm font-medium">
              {t("providers.apiKeyEnv")}
            </label>
            <Input
              id="provider-api-key-env"
              value={apiKeyEnv}
              onChange={(event) => setApiKeyEnv(event.target.value)}
              placeholder="ANTHROPIC_API_KEY"
              data-slot="provider-api-key-env"
              className="font-mono"
            />
            <span className="text-xs text-muted-foreground">{t("providers.apiKeyEnvHint")}</span>
          </div>

          <div className="grid gap-1.5">
            <div className="flex flex-wrap items-center justify-between gap-1">
              <label htmlFor="provider-api-key" className="text-sm font-medium">
                {t("providers.apiKey")}
              </label>
              {editing && storedHint && !clearKey ? (
                <button
                  type="button"
                  className="text-xs text-destructive hover:underline"
                  data-slot="provider-api-key-clear"
                  onClick={() => {
                    setClearKey(true);
                    setApiKey("");
                  }}
                >
                  {t("providers.apiKeyClear")}
                </button>
              ) : null}
            </div>
            <Input
              id="provider-api-key"
              value={apiKey}
              onChange={(event) => {
                setApiKey(event.target.value);
                // Nhập lại text sau khi đánh dấu xoá ⇒ ý định đổi thành dán key mới.
                if (clearKey) setClearKey(false);
              }}
              type="password"
              autoComplete="off"
              spellCheck={false}
              disabled={clearKey}
              placeholder={
                clearKey
                  ? t("providers.apiKeyCleared")
                  : storedHint
                    ? t("providers.apiKeySavedPlaceholder", { hint: storedHint })
                    : apiKeyEnv.trim() === ""
                      ? t("providers.apiKeyPastePlaceholder")
                      : t("providers.apiKeyEnvPlaceholder", { env: apiKeyEnv.trim() })
              }
              data-slot="provider-api-key"
              className="font-mono"
            />
            <span className="text-xs text-muted-foreground">{t("providers.apiKeyHint")}</span>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-1.5">
              <label htmlFor="provider-base-url" className="text-sm font-medium">
                {t("providers.baseUrl")}
              </label>
              <Input
                id="provider-base-url"
                value={baseUrl}
                onChange={(event) => setBaseUrl(event.target.value)}
                placeholder="https://…"
                data-slot="provider-base-url"
                className="font-mono"
              />
              <span className="text-xs text-muted-foreground">{t("providers.baseUrlHint")}</span>
            </div>
            <div className="grid gap-1.5">
              <label htmlFor="provider-max-tokens" className="text-sm font-medium">
                {t("providers.maxTokens")}
              </label>
              <Input
                id="provider-max-tokens"
                value={maxTokens}
                onChange={(event) => setMaxTokens(event.target.value)}
                type="number"
                min={1}
                step={1}
                inputMode="numeric"
                data-slot="provider-max-tokens"
              />
              <span className="text-xs text-muted-foreground">{t("providers.maxTokensHint")}</span>
            </div>
          </div>

          {submitError ? (
            <p role="alert" className="flex items-center gap-2 text-sm text-destructive">
              <CircleAlertIcon className="size-4 shrink-0" aria-hidden="true" />
              {submitError}
            </p>
          ) : null}

          <DialogFooter>
            <Button type="button" variant="outline" disabled={pending} onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={pending}>
              {pending ? t("common.loading") : t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
