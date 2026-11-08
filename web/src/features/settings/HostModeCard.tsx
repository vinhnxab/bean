import { useState } from "react";

import { ApiRequestError } from "@/api/client";
import type { HostModeDto } from "@/api/generated/HostModeDto";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useHostMode, useSetHostMode } from "@/features/settings/queries";
import { useI18n } from "@/i18n";

/**
 * Thẻ **Host mode** (D22.2) trong mục Nâng cao: bật/tắt `run_shell` chạy trực
 * tiếp trên máy thay vì sandbox, ngay trên UI.
 *
 * # Ranh giới bảo mật
 *
 * `security.sandbox.mode` là khoá tầng B + `apply = Restart` — khác LAN
 * (D21.7), **không** hot-apply: sandbox được dựng một lần lúc khởi động. Bật là
 * hành vi nguy hiểm: mọi lệnh shell trở thành `Dangerous` (luôn hỏi, không có
 * "cho phép trong phiên"). Step-up dùng **mật khẩu tài khoản OS** của tiến trình
 * Bean — máy không đặt mật khẩu thì bỏ qua (`os_password_required = false`),
 * không inspect được thì luôn hỏi (fail-closed). Mode mới chỉ hiệu lực sau
 * restart — card hiển thị gợi ý từ cặp `mode`/`running_mode` của GET, banner
 * restart của trang đọc từ bảng thiết lập (invalidate `settings` khi ghi).
 */
export function HostModeCard() {
  const { t } = useI18n();
  const hostMode = useHostMode();
  const data = hostMode.data;

  return (
    <div className="mt-4 border border-border rounded-lg p-4">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h3 className="text-sm font-medium">{t("settings.hostMode.title")}</h3>
          <p className="mt-1 text-xs text-muted-foreground">{t("settings.hostMode.description")}</p>
        </div>
        {data ? <HostModeToggle data={data} /> : null}
      </div>

      {hostMode.isPending ? (
        <p className="mt-3 text-xs text-muted-foreground">{t("common.loading")}</p>
      ) : null}

      {hostMode.isError ? (
        <div className="mt-3 flex items-center gap-2">
          <span className="text-xs text-destructive font-medium" role="alert">
            {t("settings.hostMode.loadFailed")}
          </span>
          <Button type="button" variant="outline" size="sm" onClick={() => void hostMode.refetch()}>
            {t("common.retry")}
          </Button>
        </div>
      ) : null}

      {data ? (
        <>
          <p className="mt-3 text-xs text-muted-foreground" data-testid="host-mode-status">
            {data.mode === "host" ? t("settings.hostMode.statusHost") : t("settings.hostMode.statusDocker")}
          </p>
          {data.pending ? (
            <p
              className="mt-2 rounded-lg border border-need/50 bg-need/10 px-3 py-2 text-xs text-need"
              role="status"
            >
              {t("settings.hostMode.pendingHint")}
            </p>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

/**
 * Công tắc + form xác nhận step-up mật khẩu máy. Khác LAN: Apply **không** bị
 * khoá khi ô mật khẩu trống — server là nơi quyết định có cần step-up hay không
 * (máy không đặt mật khẩu, hoặc no-op khi mode đĩa đã trùng), client chỉ chuyển
 * tiếp và hiển thị lỗi 401 `os_password_required`/`invalid_os_password` tại form.
 */
function HostModeToggle({ data }: { data: HostModeDto }) {
  const { t } = useI18n();
  const setHostMode = useSetHostMode();
  // `confirming = null` là trạng thái đồng bộ với server; `true/false` là bản
  // nháp chưa gửi — user xác nhận rồi bấm Apply mới ghi đĩa.
  const [confirming, setConfirming] = useState<boolean | null>(null);
  const [password, setPassword] = useState("");

  const hostEnabled = data.mode === "host";
  const draft = confirming ?? hostEnabled;
  const dirty = confirming !== null && confirming !== hostEnabled;
  const pending = setHostMode.isPending;

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (pending || !dirty) return;
    setHostMode.mutate(
      // Máy không đặt mật khẩu ⇒ `os_password: null`; server tự chấm điểm
      // step-up (thiếu ⇒ 401 `os_password_required`).
      { enabled: confirming === true, os_password: password === "" ? null : password },
      {
        onSuccess: () => {
          setConfirming(null);
          setPassword("");
        },
      },
    );
  };

  const saveError = setHostMode.isError
    ? setHostMode.error instanceof ApiRequestError && setHostMode.error.code !== "http_error"
      ? setHostMode.error.message
      : t("settings.hostMode.saveFailed")
    : null;

  return (
    <div className="flex flex-col items-end gap-2">
      <Switch
        checked={draft}
        disabled={pending}
        aria-label={t("settings.hostMode.toggleAria")}
        onCheckedChange={(enabled) => {
          if (enabled === hostEnabled) {
            setConfirming(null);
            setPassword("");
          } else {
            setConfirming(enabled);
            setPassword("");
          }
        }}
      />

      {confirming !== null ? (
        <form className="grid gap-2 w-full sm:w-72" onSubmit={submit}>
          <p className="text-xs text-muted-foreground">
            {dirty
              ? t(confirming ? "settings.hostMode.enableHint" : "settings.hostMode.disableHint")
              : t("settings.hostMode.noChangeHint")}
          </p>
          {data.os_password_required ? (
            <Input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              placeholder={t("settings.hostMode.passwordPlaceholder")}
              aria-label={t("settings.hostMode.passwordAria")}
              disabled={pending}
            />
          ) : null}
          <div className="flex items-center gap-2">
            <Button type="submit" size="sm" disabled={!dirty || pending}>
              {t("settings.hostMode.apply")}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={pending}
              onClick={() => {
                setConfirming(null);
                setPassword("");
              }}
            >
              {t("settings.hostMode.cancel")}
            </Button>
            {saveError ? (
              <span className="text-xs text-destructive font-medium" role="alert">
                {saveError}
              </span>
            ) : null}
          </div>
        </form>
      ) : null}
    </div>
  );
}
