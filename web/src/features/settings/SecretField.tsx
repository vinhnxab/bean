"use client";

import { CircleAlertIcon, EyeIcon, EyeOffIcon } from "lucide-react";
import { useState } from "react";

import type { SecretSlotDto } from "@/api/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useDeleteSecret, usePutSecret } from "@/features/settings/queries";
import { useI18n } from "@/i18n";
import type { MessageKey } from "@/i18n/vi";

/**
 * Ô nhập secret write-only theo đúng mục §8.2 của M29:
 *
 * - Hiển thị **chỉ cờ** `set`/`source` — giá trị secret không bao giờ hiện ở đây.
 * - Ô nhập `type=password`, `autocomplete="new-password"`, **không bao giờ điền sẵn**.
 * - Sau khi lưu/xoá: xoá khỏi state và DOM (không giữ lâu dài, không vào zustand/query cache).
 * - Nguồn `env` (biến môi trường ghi đè secrets.env) → khóa nút kèm giải thích.
 * - Xoá chỉ cần mật khẩu hiện tại (step-up ở server), không cần nhập giá trị.
 */
export function SecretField({ slot, disabled = false }: { slot: SecretSlotDto; disabled?: boolean }) {
  const { t } = useI18n();
  const putSecret = usePutSecret();
  const deleteSecret = useDeleteSecret();

  const lockedByEnv = slot.source === "env" || slot.shadowed_by_env;
  const fieldDisabled = disabled || lockedByEnv;

  const [mode, setMode] = useState<"idle" | "enter" | "confirmDelete">("idle");
  const [value, setValue] = useState("");
  const [password, setPassword] = useState("");
  const [revealed, setRevealed] = useState(false);
  const [status, setStatus] = useState<"idle" | "saving" | "saved" | "deleted" | "error">("idle");
  const [error, setError] = useState<string | null>(null);

  const canSave =
    !putSecret.isPending &&
    !deleteSecret.isPending &&
    value.length > 0 &&
    password.length > 0 &&
    !lockedByEnv;

  const handleSave = (event: React.FormEvent) => {
    event.preventDefault();
    if (!canSave) return;
    setStatus("saving");
    setError(null);
    void putSecret
      .mutateAsync({ name: slot.name, payload: { value, password } })
      .then(() => {
        setValue("");
        setPassword("");
        setMode("idle");
        setStatus("saved");
        setTimeout(() => setStatus("idle"), 1500);
      })
      .catch((err: unknown) => {
        setStatus("error");
        setError(err instanceof Error ? err.message : t("secret.saveError"));
      });
  };

  const handleDeleteConfirm = (event: React.FormEvent) => {
    event.preventDefault();
    if (deleteSecret.isPending || lockedByEnv || !slot.set || !password) return;
    setStatus("saving");
    setError(null);
    void deleteSecret
      .mutateAsync({ name: slot.name, password })
      .then(() => {
        setPassword("");
        setMode("idle");
        setStatus("deleted");
        setTimeout(() => setStatus("idle"), 1500);
      })
      .catch((err: unknown) => {
        setStatus("error");
        setError(err instanceof Error ? err.message : t("secret.deleteError"));
      });
  };
  const label = slot.set ? t("secret.slotSet") : t("secret.slotNotSet");
  const sourceKey: MessageKey =
    slot.source === "env"
      ? "secret.source.env"
      : slot.source === "secrets_env"
        ? "secret.source.secrets_env"
        : "secret.source.unset";
  const sourceLabel = t(sourceKey);

  return (
    <div className="mt-4 grid items-center gap-3 border rounded-lg p-4 sm:grid-cols-[1fr,auto]">
      <div className="min-w-0">
        <p className="text-sm font-medium">{slot.name}</p>
        <p className="mt-0.5 text-xs text-muted-foreground truncate">{slot.label}</p>
      </div>

      <div className="flex items-center gap-2 text-sm font-medium">
        {slot.set ? (
          <>
            <span>{label}</span>
            <span className="ml-1 rounded-full border px-1.5 py-0 text-[10px] leading-none text-muted-foreground">
              {sourceLabel}
            </span>
          </>
        ) : (
          <span className="text-muted-foreground">{label}</span>
        )}
        {slot.shadowed_by_env && (
          <span
            className="ml-1 flex items-center gap-1 text-xs text-amber-700 dark:text-amber-400"
            title={t("secret.shadowedHint")}
          >
            <CircleAlertIcon className="size-3" aria-hidden="true" />
          </span>
        )}
      </div>

      {lockedByEnv && <p className="mt-1 text-xs text-muted-foreground">{t("secret.envLocked")}</p>}

      <div className="flex items-center gap-2">
        {status === "saved" ? (
          <span role="status" className="text-xs text-muted-foreground">
            {t("secret.saved")}
          </span>
        ) : status === "deleted" ? (
          <span role="status" className="text-xs text-muted-foreground">
            {t("secret.deleted")}
          </span>
        ) : null}

        {mode === "idle" ? (
          <>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={fieldDisabled || putSecret.isPending || deleteSecret.isPending}
              onClick={() => {
                setMode("enter");
                setError(null);
              }}
            >
              {slot.set ? t("secret.replace") : t("secret.add")}
            </Button>
            {slot.set ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={fieldDisabled || putSecret.isPending || deleteSecret.isPending}
                onClick={() => setMode("confirmDelete")}
              >
                {t("secret.delete")}
              </Button>
            ) : null}
          </>
        ) : null}

        {lockedByEnv && (
          <span
            className="ml-1 flex items-center gap-1 text-xs text-amber-700 dark:text-amber-400"
            title={t("secret.shadowedHint")}
          >
            <CircleAlertIcon className="size-3" aria-hidden="true" />
          </span>
        )}
      </div>

      {mode === "enter" && (
        <form
          id={`secret-form-${slot.name}`}
          className="grid col-span-full gap-3 sm:col-span-2 border-t border-border pt-3 sm:pt-0"
          onSubmit={handleSave}
        >
          <div className="grid gap-1.5 mt-3">
            <label htmlFor={`secret-value-${slot.name}`} className="text-sm font-medium">
              {t("secret.enter")}
            </label>
            <Input
              id={`secret-value-${slot.name}`}
              type={revealed ? "text" : "password"}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              autoComplete="new-password"
              placeholder={t("secret.enterPlaceholder")}
              disabled={fieldDisabled}
              className="pr-10"
            />
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="absolute inset-y-0 right-0 h-full w-9 text-muted-foreground hover:text-foreground"
              aria-label={t(revealed ? "password.hide" : "password.show")}
              aria-pressed={revealed}
              disabled={fieldDisabled}
              tabIndex={-1}
              onClick={() => setRevealed((v) => !v)}
            >
              {revealed ? (
                <EyeOffIcon className="size-4" aria-hidden="true" />
              ) : (
                <EyeIcon className="size-4" aria-hidden="true" />
              )}
            </Button>
          </div>

          <div className="grid gap-1.5">
            <label htmlFor={`secret-password-${slot.name}`} className="text-sm font-medium">
              {t("secret.password")}
            </label>
            <Input
              id={`secret-password-${slot.name}`}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              placeholder={t("secret.passwordPlaceholder")}
              disabled={fieldDisabled}
            />
          </div>

          <div className="flex items-center gap-2">
            <Button type="submit" disabled={!canSave}>
              {t("secret.save")}
            </Button>
            <Button type="button" variant="outline" size="sm" onClick={() => setMode("idle")}>
              {t("secret.cancel")}
            </Button>
            {error ? (
              <span className="text-xs text-destructive font-medium" role="alert">
                {error}
              </span>
            ) : null}
          </div>
        </form>
      )}

      {mode === "confirmDelete" && (
        <div className="grid col-span-full gap-3 sm:col-span-2 border-t border-border pt-3 sm:pt-0">
          <div className="grid gap-1.5">
            <label htmlFor={`secret-delete-password-${slot.name}`} className="text-sm font-medium">
              {t("secret.password")}
            </label>
            <Input
              id={`secret-delete-password-${slot.name}`}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              placeholder={t("secret.passwordPlaceholder")}
              disabled={fieldDisabled}
            />
          </div>

          <form onSubmit={handleDeleteConfirm} className="flex items-center gap-2">
            <Button type="submit" disabled={!password || deleteSecret.isPending || lockedByEnv}>
              {t("secret.delete")}
            </Button>
            <Button type="button" variant="outline" size="sm" onClick={() => setMode("idle")}>
              {t("secret.cancel")}
            </Button>
            {error ? (
              <span className="text-xs text-destructive font-medium" role="alert">
                {error}
              </span>
            ) : null}
          </form>
        </div>
      )}
    </div>
  );
}
