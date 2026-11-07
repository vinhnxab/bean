import { useState } from "react";

import type { LanAccessDto } from "@/api/bindings";
import { ApiRequestError } from "@/api/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useLanAccess, useSetLanAccess } from "@/features/settings/queries";
import { useI18n } from "@/i18n";

/**
 * Thẻ **LAN Access** (D21.x) trong mục Nâng cao: bật/tắt truy cập Bean từ mạng
 * nội bộ ngay trên UI thay vì sửa tay `bean.toml` + restart.
 *
 * # Ranh giới bảo mật
 *
 * Ba khoá bị sửa (`web.allow_remote`, `web.bind`, `web.public_origin`) là tầng B
 * — ngoại lệ có kiểm soát so với M29: chỉ admin, **step-up mật khẩu** như secrets
 * (D19.5), audit từng khoá, và hiệu lực sau `scripts/beanctl restart` (banner
 * pending restart của panel tự hiện qua invalidation `settings`).
 */
export function LanAccessCard() {
  const { t } = useI18n();
  const lan = useLanAccess();
  const data = lan.data;

  return (
    <div className="mt-4 border border-border rounded-lg p-4">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h3 className="text-sm font-medium">{t("settings.lanAccess.title")}</h3>
          <p className="mt-1 text-xs text-muted-foreground">{t("settings.lanAccess.description")}</p>
        </div>
        {data ? <LanAccessToggle data={data} /> : null}
      </div>

      {lan.isPending ? <p className="mt-3 text-xs text-muted-foreground">{t("common.loading")}</p> : null}

      {lan.isError ? (
        <div className="mt-3 flex items-center gap-2">
          <span className="text-xs text-destructive font-medium" role="alert">
            {t("settings.lanAccess.loadFailed")}
          </span>
          <Button type="button" variant="outline" size="sm" onClick={() => void lan.refetch()}>
            {t("common.retry")}
          </Button>
        </div>
      ) : null}

      {data ? (
        <p className="mt-3 text-xs text-muted-foreground" data-testid="lan-access-status">
          {data.mode === "lan"
            ? t("settings.lanAccess.statusLan", { url: data.url ?? "" })
            : t("settings.lanAccess.statusLoopback")}
        </p>
      ) : null}
    </div>
  );
}

/**
 * Công tắc + form xác nhận step-up. `key={data.mode}` ở component cha remount
 * form khi trạng thái hiệu lực trên server đổi — bản nháp mật khẩu về rỗng sau
 * mỗi lần ghi thành công (giống `VoiceForm`).
 */
function LanAccessToggle({ data }: { data: LanAccessDto }) {
  const { t } = useI18n();
  const setLanAccess = useSetLanAccess();
  // `confirming = null` là trạng thái đồng bộ với server; `true/false` là bản
  // nháp chưa gửi — user phải nhập mật khẩu mới áp dụng.
  const [confirming, setConfirming] = useState<boolean | null>(null);
  const [password, setPassword] = useState("");

  const lanEnabled = data.mode === "lan";
  const draft = confirming ?? lanEnabled;
  const dirty = confirming !== null && confirming !== lanEnabled;
  const pending = setLanAccess.isPending;
  const targetUrl =
    confirming === true
      ? data.lan_ip
        ? `http://${data.lan_ip}:${data.port}`
        : null
      : `http://127.0.0.1:${data.port}`;

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (pending || !dirty || password === "") return;
    setLanAccess.mutate(
      { enabled: confirming === true, password },
      {
        onSuccess: () => {
          setConfirming(null);
          setPassword("");
        },
      },
    );
  };

  const saveError = setLanAccess.isError
    ? setLanAccess.error instanceof ApiRequestError && setLanAccess.error.code !== "http_error"
      ? setLanAccess.error.message
      : t("settings.lanAccess.saveFailed")
    : null;

  return (
    <div className="flex flex-col items-end gap-2">
      <Switch
        checked={draft}
        disabled={pending}
        aria-label={t("settings.lanAccess.toggleAria")}
        onCheckedChange={(enabled) => {
          if (enabled === lanEnabled) {
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
            {dirty && targetUrl
              ? t(confirming ? "settings.lanAccess.enableHint" : "settings.lanAccess.disableHint", {
                  url: targetUrl,
                })
              : t("settings.lanAccess.noChangeHint")}
          </p>
          <Input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="current-password"
            placeholder={t("settings.lanAccess.passwordPlaceholder")}
            aria-label={t("settings.lanAccess.passwordAria")}
            disabled={pending}
          />
          <div className="flex items-center gap-2">
            <Button type="submit" size="sm" disabled={!dirty || password === "" || pending}>
              {t("settings.lanAccess.apply")}
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
              {t("settings.lanAccess.cancel")}
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
