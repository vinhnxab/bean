import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Panel } from "@/components/ui/Page";
import { Switch } from "@/components/ui/switch";
import { queryKeys } from "@/features/auth/queries";
import { usePushStatus, usePushTest } from "@/features/settings/queries";
import { useI18n } from "@/i18n";
import {
  disableSound,
  enableSound,
  getAttentionName,
  isSoundEnabled,
  setAttentionName,
  testSoundAttention,
} from "@/lib/attention";
import {
  disableNotifications,
  enableNotifications,
  isNotifyEnabled,
  notificationPermission,
} from "@/lib/notify";
import { disablePush, enablePush, isPushEnabled, pushSupported } from "@/lib/push";

/**
 * Mục **Thông báo** trong Cài đặt: cấu hình ba kênh báo hiệu khi tab Bean ẩn —
 * thông báo hệ thống (trong tab), Web Push (khi tab đã đóng) và âm thanh gọi
 * tên — cùng tên được gọi ("Vinh Mr ơi, Bean trả lời xong rồi.").
 *
 * # Vì sao hai kênh đầu là localStorage còn Web Push qua REST
 *
 * Thông báo hệ thống và âm thanh là tuỳ chọn của **thiết bị này** (máy tính bật
 * chuông, điện thoại để im) — cùng logic với chủ đề; server không cần biết.
 * Web Push khác: server giữ subscription để chủ động đẩy tin, nên đăng ký/gỡ
 * đi qua REST và panel dùng `usePushStatus` (admin) hiển thị thiết bị.
 */
export function NotificationsPanel() {
  const { t, lang } = useI18n();

  // Trạng thái khởi tạo một lần từ bộ nhớ thiết bị; mutate qua từng handler —
  // không có nguồn dữ liệu bên ngoài nào cần phản chiếu.
  const [notifyEnabled, setNotifyEnabled] = useState<boolean>(() => isNotifyEnabled());
  const [permission, setPermission] = useState(() => notificationPermission());
  const [soundEnabled, setSoundEnabled] = useState<boolean>(() => isSoundEnabled());
  const [name, setName] = useState<string>(() => getAttentionName());
  const [savedName, setSavedName] = useState<string>(() => getAttentionName());

  // Push: `null` = đang dò subscription của trình duyệt này; kết quả dò là dữ
  // liệu server+trình duyệt, không đọc được đồng bộ nên nạp một lần khi mount.
  const [pushEnabled, setPushEnabled] = useState<boolean | null>(null);
  const [pushBusy, setPushBusy] = useState(false);
  const [pushError, setPushError] = useState(false);
  const pushQuery = usePushStatus();
  const pushTestMutation = usePushTest();
  const queryClient = useQueryClient();
  const pushOk = pushSupported();

  useEffect(() => {
    if (!pushOk) return;
    let cancelled = false;
    void isPushEnabled().then((on) => {
      if (!cancelled) setPushEnabled(on);
    });
    return () => {
      cancelled = true;
    };
  }, [pushOk]);

  async function toggleNotifications(next: boolean): Promise<void> {
    if (!next) {
      disableNotifications();
      setNotifyEnabled(false);
      return;
    }
    const result = await enableNotifications();
    setPermission(result);
    setNotifyEnabled(result === "granted");
  }

  function toggleSound(next: boolean): void {
    if (next) {
      enableSound();
      setSoundEnabled(true);
      return;
    }
    disableSound();
    setSoundEnabled(false);
  }

  async function togglePush(next: boolean): Promise<void> {
    setPushError(false);
    setPushBusy(true);
    try {
      if (next) {
        const result = await enablePush();
        if (result === "permission_denied") {
          // requestPermission vừa chạy xong với kết quả từ chối — quyền toàn cục
          // đã đổi, phản chiếu luôn sang hàng thông báo trong tab.
          setPermission("denied");
        }
        setPushEnabled(result === "enabled");
      } else {
        await disablePush();
        setPushEnabled(false);
      }
      // Danh sách thiết bị phía server vừa thay đổi (UPSERT/XOÁ) — làm mới.
      void queryClient.invalidateQueries({ queryKey: queryKeys.pushStatus });
    } catch {
      // Thất bại (context không secure, server lỗi…): dò lại trạng thái thật
      // thay vì đoán — subscription có thể vẫn nguyên vẹn.
      setPushError(true);
      setPushEnabled(await isPushEnabled());
    } finally {
      setPushBusy(false);
    }
  }

  function saveName(): void {
    setAttentionName(name);
    setSavedName(getAttentionName());
  }

  function testSound(): void {
    // Lưu bản nháp trước khi phát thử để câu đọc đúng tên đang gõ — người dùng
    // thường vừa đặt tên vừa bấm thử, không muốn phải bấm Lưu trước.
    setAttentionName(name);
    setSavedName(getAttentionName());
    testSoundAttention(getAttentionName(), t, lang);
  }

  const notifyBlocked = permission === "denied";
  const notifyUnsupported = permission === "unsupported";
  const notifyState = notifyUnsupported
    ? t("notify.panel.unsupported")
    : notifyBlocked
      ? t("notify.blockedHint")
      : notifyEnabled
        ? t("notify.panel.on")
        : t("notify.panel.off");
  const soundState = soundEnabled ? t("notify.panel.on") : t("notify.panel.off");
  const nameDirty = name.trim() !== savedName;
  const pushState = !pushOk
    ? t("notify.panel.pushUnsupported")
    : pushEnabled === null
      ? t("notify.panel.pushChecking")
      : pushEnabled
        ? t("notify.panel.on")
        : t("notify.panel.off");
  const pushDevices = pushQuery.data?.subscriptions ?? [];
  const pushTestResult = pushTestMutation.data;

  return (
    <Panel>
      <h2 className="font-semibold">{t("notify.panel.title")}</h2>
      <p className="mt-1 text-sm text-muted-foreground">{t("notify.panel.hint")}</p>

      <div className="mt-5 grid gap-4 border-t border-border pt-4">
        <div className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium">{t("notify.panel.desktop")}</span>
            <span className="text-xs text-muted-foreground">{notifyState}</span>
          </div>
          <Switch
            checked={notifyEnabled}
            disabled={notifyUnsupported || notifyBlocked}
            aria-label={t("notify.panel.desktop")}
            onCheckedChange={(next) => void toggleNotifications(next)}
          />
        </div>

        <div className="grid gap-1.5">
          <div className="flex items-center justify-between gap-4">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium">{t("notify.panel.push")}</span>
              <span className="text-xs text-muted-foreground">{pushState}</span>
            </div>
            <Switch
              checked={pushEnabled === true}
              disabled={!pushOk || pushEnabled === null || pushBusy}
              aria-label={t("notify.panel.push")}
              onCheckedChange={(next) => void togglePush(next)}
            />
          </div>
          <p className="text-xs text-muted-foreground">{t("notify.panel.pushHint")}</p>
          {pushError ? <p className="text-xs text-alert">{t("notify.panel.pushError")}</p> : null}
          {pushQuery.isSuccess ? (
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                disabled={pushEnabled !== true || pushTestMutation.isPending}
                onClick={() => pushTestMutation.mutate()}
              >
                {t("notify.panel.pushTest")}
              </Button>
              {pushTestResult ? (
                <span role="status" className="text-xs text-muted-foreground">
                  {t("notify.panel.pushTestResult", {
                    delivered: pushTestResult.delivered,
                    failed: pushTestResult.failed,
                    removed: pushTestResult.removed,
                  })}
                </span>
              ) : null}
            </div>
          ) : null}
          {pushDevices.length > 0 ? (
            <div className="grid gap-1">
              <span className="text-xs font-medium text-muted-foreground">
                {t("notify.panel.pushDevices")}
              </span>
              {pushDevices.map((device) => (
                <div
                  key={device.label}
                  className="flex items-center justify-between gap-2 text-xs text-muted-foreground"
                >
                  <span className="truncate" title={device.label}>
                    {device.label}
                    {device.user_agent ? ` · ${device.user_agent}` : ""}
                  </span>
                  {device.failure_count > 0 ? (
                    <span className="shrink-0 text-alert">
                      {t("notify.panel.pushFailures", { count: device.failure_count })}
                    </span>
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
        </div>

        <div className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium">{t("notify.panel.sound")}</span>
            <span className="text-xs text-muted-foreground">{soundState}</span>
          </div>
          <Switch checked={soundEnabled} aria-label={t("notify.panel.sound")} onCheckedChange={toggleSound} />
        </div>

        <form
          className="grid gap-1.5"
          onSubmit={(event) => {
            event.preventDefault();
            saveName();
          }}
        >
          <label htmlFor="notify-name" className="text-sm font-medium">
            {t("notify.panel.name")}
          </label>
          <div className="flex flex-wrap items-center gap-2">
            <Input
              id="notify-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder={t("notify.panel.namePlaceholder")}
              autoComplete="off"
              className="w-full sm:w-72"
            />
            <Button type="submit" disabled={!nameDirty}>
              {t("common.save")}
            </Button>
            <Button type="button" variant="outline" disabled={!soundEnabled} onClick={testSound}>
              {t("notify.panel.test")}
            </Button>
            {!nameDirty && name.trim() !== "" ? (
              <span role="status" className="text-xs text-muted-foreground">
                {t("notify.panel.nameSaved")}
              </span>
            ) : null}
          </div>
          <p className="text-xs text-muted-foreground">{t("notify.panel.nameHint")}</p>
        </form>
      </div>
    </Panel>
  );
}
