import { CircleAlertIcon, EyeIcon, EyeOffIcon } from "lucide-react";
import { useState } from "react";

import { ApiRequestError } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Panel } from "@/components/ui/Page";
import { useChangePassword } from "@/features/settings/queries";
import { useI18n } from "@/i18n";

/**
 * Input mật khẩu có nút show/hide.
 */
function PasswordInput({
  id,
  value,
  onChange,
  autoComplete,
  disabled,
}: {
  id: string;
  value: string;
  onChange: (v: string) => void;
  autoComplete?: string;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  const [visible, setVisible] = useState(false);

  return (
    <div className="relative w-full sm:w-72">
      <Input
        id={id}
        type={visible ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        autoComplete={autoComplete}
        disabled={disabled}
        className="pr-9"
      />
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="absolute inset-y-0 right-0 h-full w-9 text-muted-foreground hover:text-foreground"
        aria-label={t(visible ? "password.hide" : "password.show")}
        aria-pressed={visible}
        tabIndex={-1}
        onClick={() => setVisible((v) => !v)}
        disabled={disabled}
      >
        {visible ? (
          <EyeOffIcon className="size-4" aria-hidden="true" />
        ) : (
          <EyeIcon className="size-4" aria-hidden="true" />
        )}
      </Button>
    </div>
  );
}

/**
 * Mục **Đổi mật khẩu** trong Cài đặt.
 *
 * Gửi `POST /api/auth/change-password`. Server xác minh mật khẩu hiện tại,
 * ghi hash mới, thu hồi toàn bộ session — client sẽ bị đăng xuất sau ~1.5s.
 */
export function PasswordPanel() {
  const { t } = useI18n();
  const changePassword = useChangePassword();

  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");

  const pending = changePassword.isPending || changePassword.isSuccess;

  const validationError: string | null = (() => {
    if (next.length > 0 && next.length < 12) return t("password.minLength");
    if (confirm.length > 0 && next !== confirm) return t("password.mismatch");
    return null;
  })();

  const canSubmit =
    !pending && current.length > 0 && next.length >= 12 && next === confirm && validationError === null;

  const serverError: string | null = (() => {
    if (!changePassword.isError) return null;
    const err = changePassword.error;
    if (err instanceof ApiRequestError && err.code === "invalid_credentials") {
      return t("password.wrongCurrent");
    }
    return t("password.error");
  })();

  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!canSubmit) return;
    changePassword.mutate({ current_password: current, new_password: next });
  };

  return (
    <Panel>
        <h2 className="font-semibold">{t("password.panelTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("password.panelHint")}</p>

        <form className="mt-5 grid gap-4 border-t border-border pt-4" onSubmit={handleSubmit}>
          {/* Mật khẩu hiện tại */}
          <div className="grid gap-1.5">
            <label htmlFor="pwd-current" className="text-sm font-medium">
              {t("password.current")}
            </label>
            <PasswordInput
              id="pwd-current"
              value={current}
              onChange={setCurrent}
              autoComplete="current-password"
              disabled={pending}
            />
          </div>

          {/* Mật khẩu mới */}
          <div className="grid gap-1.5">
            <label htmlFor="pwd-new" className="text-sm font-medium">
              {t("password.new")}
            </label>
            <PasswordInput
              id="pwd-new"
              value={next}
              onChange={setNext}
              autoComplete="new-password"
              disabled={pending}
            />
          </div>

          {/* Xác nhận mật khẩu mới */}
          <div className="grid gap-1.5">
            <label htmlFor="pwd-confirm" className="text-sm font-medium">
              {t("password.confirm")}
            </label>
            <PasswordInput
              id="pwd-confirm"
              value={confirm}
              onChange={setConfirm}
              autoComplete="new-password"
              disabled={pending}
            />
          </div>

          {/* Nút submit + thông báo */}
          <div className="flex flex-wrap items-center gap-3">
            <Button type="submit" disabled={!canSubmit}>
              {t("password.submit")}
            </Button>
            {changePassword.isSuccess ? (
              <span role="status" className="text-xs text-muted-foreground">
                {t("password.success")}
              </span>
            ) : null}
          </div>

          {/* Lỗi validation client */}
          {validationError ? (
            <p className="flex items-center gap-1.5 text-sm font-medium text-destructive" role="alert">
              <CircleAlertIcon className="size-3.5 shrink-0" aria-hidden="true" />
              {validationError}
            </p>
          ) : null}

          {/* Lỗi server */}
          {serverError ? (
            <p className="flex items-center gap-1.5 text-sm font-medium text-destructive" role="alert">
              <CircleAlertIcon className="size-3.5 shrink-0" aria-hidden="true" />
              {serverError}
            </p>
          ) : null}
        </form>
      </Panel>
  );
}
