import { BellIcon, BellRingIcon } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n";
import {
  disableNotifications,
  enableNotifications,
  isNotifyEnabled,
  notificationPermission,
} from "@/lib/notify";

/**
 * Nút bật/tắt thông báo hệ thống khi tab Bean đang ẩn.
 *
 * # Vì sao là nút bấm chứ không chuyển trong Cài đặt
 *
 * Trình duyệt chỉ cho xin quyền thông báo trong ngữ cảnh người dùng chủ động —
 * bấm nút chính là ngữ cảnh đó. Đặt cạnh nút chủ đề trên TopNav: cùng là lựa
 * chọn toàn cục của thiết bị, cùng kiểu icon trơn với nhãn trợ năng đủ nghĩa.
 *
 * # Trạng thái
 *
 * `aria-pressed` đúng nghĩa bật/tắt. Bật mà trình duyệt từ chối quyền thì lựa
 * chọn không được ghi (`enableNotifications` chỉ ghi khi `granted`) — nút quay
 * về tắt và `title` nói rõ lý do thay vì im lặng khiến người dùng bấm hoài
 * không hiểu sao không được.
 */
export function NotifyToggle({ className = "" }: { className?: string }) {
  const { t } = useI18n();
  const [enabled, setEnabled] = useState<boolean>(() => isNotifyEnabled());
  const [permission, setPermission] = useState(() => notificationPermission());

  async function toggle(): Promise<void> {
    if (enabled) {
      disableNotifications();
      setEnabled(false);
      return;
    }
    const next = await enableNotifications();
    setPermission(next);
    setEnabled(next === "granted");
  }

  const blocked = permission === "denied";
  const label = enabled ? t("notify.disable") : blocked ? t("notify.blockedHint") : t("notify.enable");

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      onClick={() => void toggle()}
      aria-pressed={enabled}
      aria-label={label}
      title={label}
      data-slot="notify-toggle"
      data-notify-state={enabled ? "on" : "off"}
      className={className}
    >
      {enabled ? <BellRingIcon /> : <BellIcon />}
    </Button>
  );
}
