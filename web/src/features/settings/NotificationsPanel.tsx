import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Panel } from "@/components/ui/Page";
import { Switch } from "@/components/ui/switch";
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

/**
 * Mục **Thông báo** trong Cài đặt: cấu hình hai kênh báo hiệu khi tab Bean ẩn —
 * thông báo hệ thống và âm thanh gọi tên — cùng tên được gọi ("Vinh Mr ơi, Bean
 * trả lời xong rồi.").
 *
 * # Vì sao là localStorage chứ không REST
 *
 * Cả hai là tuỳ chọn của **thiết bị này** (máy tính bật chuông, điện thoại để
 * im) — cùng logic với chủ đề; server không cần biết. Panel phơi chúng ra để
 * người dùng tìm được chỗ đặt tên gọi: nút loa/chuông trên TopNav chỉ bật/tắt.
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
