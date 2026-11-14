import { Volume2Icon, VolumeXIcon } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n";
import {
  disableSound,
  enableSound,
  getAttentionName,
  isSoundEnabled,
  testSoundAttention,
} from "@/lib/attention";

/**
 * Nút bật/tắt âm thanh gọi tên (chime + speechSynthesis) khi tab Bean đang ẩn.
 *
 * # Vì sao là nút bấm chứ không chỉ có trong Cài đặt
 *
 * Trình duyệt chỉ gỡ khoá autoplay cho `AudioContext`/`speechSynthesis` sau một
 * user gesture — bấm nút chính là gesture đó: lần bật đầu phát thử một câu vừa
 * là phản hồi cho người dùng vừa là "mở khoá" cho những lần phát sau khi tab
 * ẩn. Đặt cạnh nút chuông trên TopNav: cùng là lựa chọn toàn cục của thiết bị,
 * cùng kiểu icon trơn với nhãn trợ năng đủ nghĩa.
 */
export function SoundToggle({ className = "" }: { className?: string }) {
  const { t, lang } = useI18n();
  const [enabled, setEnabled] = useState<boolean>(() => isSoundEnabled());

  function toggle(): void {
    if (enabled) {
      disableSound();
      setEnabled(false);
      return;
    }
    enableSound();
    setEnabled(true);
    // Phát thử ngay trong thao tác bấm — ngoài phản hồi, gesture này gỡ khoá
    // autoplay; thất bại (trình duyệt chặn) thì im lặng, không làm hỏng nút.
    testSoundAttention(getAttentionName(), t, lang);
  }

  const label = enabled ? t("sound.disable") : t("sound.enable");

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      onClick={toggle}
      aria-pressed={enabled}
      aria-label={label}
      title={label}
      data-slot="sound-toggle"
      data-sound-state={enabled ? "on" : "off"}
      className={className}
    >
      {enabled ? <Volume2Icon /> : <VolumeXIcon />}
    </Button>
  );
}
