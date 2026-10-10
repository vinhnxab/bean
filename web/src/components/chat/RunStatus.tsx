import gsap from "gsap";
import { useRef } from "react";

import { BeanAvatar } from "@/components/brand/BeanAvatar";
import { useI18n } from "@/i18n";
import { useGsapAnimate } from "@/lib/anim";

/**
 * Trạng thái "đang chạy" của run, thể hiện bằng ba thành phần thay cho dòng chữ
 * tĩnh "Đang xử lý" trước đây:
 *
 * - {@link ThinkingIndicator} — Bean đang suy nghĩ: avatar + nhãn + ba chấm nhảy
 *   (GSAP stagger lặp vô hạn). Hiện khi run đang chạy mà chưa có chữ streaming —
 *   lúc đó chính chữ streaming + {@link StreamingCaret} đã là tín hiệu sống.
 * - {@link QueuePill} — đứng thứ N trong hàng đợi, chấm hiệu ứng nhịp thở.
 * - {@link StreamingCaret} — chỏ nháy cuối phần chữ đang streaming.
 *
 * Mọi thành phần `role="status"` (aria-live ngầm) nên trình đọc màn hình thông
 * báo đúng một lần; chi tiết trang trí (chấm, chỏ) đều `aria-hidden`.
 */

export function ThinkingIndicator() {
  const { t } = useI18n();
  const dotsRef = useRef<HTMLSpanElement>(null);

  useGsapAnimate(() => {
    const element = dotsRef.current;
    if (!element) return;
    // Ba chấm nhảy nối tiếp nhau (stagger) rồi lặp vô hạn — dấu hiệu "vẫn đang
    // nghĩ" mềm hơn so với spinner quay.
    gsap.to(element.querySelectorAll("span"), {
      y: -4,
      duration: 0.32,
      ease: "sine.inOut",
      stagger: 0.12,
      yoyo: true,
      repeat: -1,
    });
  }, []);

  return (
    <span
      role="status"
      className="inline-flex items-center gap-2 rounded-full border border-live/40 bg-live/10 px-3 py-1.5 text-sm text-live"
    >
      {/* Avatar là chi tiết trang trí: alt="" để nhãn "Đang suy nghĩ…" không bị đọc lặp. */}
      <BeanAvatar size={18} className="ring-1 ring-live/30" />
      {t("chat.thinking")}
      <span ref={dotsRef} aria-hidden="true" className="inline-flex items-center gap-1">
        <span className="size-1.5 rounded-full bg-current" />
        <span className="size-1.5 rounded-full bg-current" />
        <span className="size-1.5 rounded-full bg-current" />
      </span>
    </span>
  );
}

export function QueuePill({ position }: { position: number | null }) {
  const { t } = useI18n();
  const dotRef = useRef<HTMLSpanElement>(null);

  useGsapAnimate(() => {
    const element = dotRef.current;
    if (!element) return;
    // Nhịp thở: mờ đi rồi sáng lại — giữ mắt biết hàng đợi vẫn đang dịch chuyển.
    gsap.to(element, {
      opacity: 0.3,
      scale: 0.7,
      duration: 0.6,
      ease: "sine.inOut",
      yoyo: true,
      repeat: -1,
    });
  }, []);

  return (
    <span
      role="status"
      className="inline-flex items-center gap-2 rounded-full border border-need/50 bg-need/10 px-3 py-1.5 text-sm font-medium text-need"
    >
      <span ref={dotRef} aria-hidden="true" className="size-2 rounded-full bg-current" />
      {position === null ? t("chat.queued") : `${t("chat.queued")} · #${position}`}
    </span>
  );
}

export function StreamingCaret() {
  const ref = useRef<HTMLSpanElement>(null);

  useGsapAnimate(() => {
    const element = ref.current;
    if (!element) return;
    gsap.to(element, {
      opacity: 0.15,
      duration: 0.5,
      ease: "sine.inOut",
      yoyo: true,
      repeat: -1,
    });
  }, []);

  return (
    <span
      ref={ref}
      aria-hidden="true"
      className="ml-0.5 inline-block h-[1em] w-[3px] translate-y-[3px] rounded-full bg-brand"
    />
  );
}
