import gsap from "gsap";
import { useEffect, useRef, useState } from "react";

import { BeanAvatar } from "@/components/brand/BeanAvatar";
import { useI18n } from "@/i18n";
import { useGsapAnimate } from "@/lib/anim";

/**
 * Trạng thái "đang chạy" của run. Quá trình trung gian (text suy nghĩ, thẻ
 * tool) không hiển thị trong dòng chat — chỉ kết quả cuối cùng mới hiện — nên
 * hai thành phần dưới đây là tín hiệu duy nhất cho biết Bean vẫn đang làm:
 *
 * - {@link ThinkingIndicator} — Bean đang suy nghĩ: avatar + nhãn + **đồng hồ
 *   thời gian đã trôi** (tick mỗi giây) + ba chấm nhảy (GSAP stagger lặp vô hạn).
 * - {@link QueuePill} — đứng thứ N trong hàng đợi, chấm hiệu ứng nhịp thở.
 *
 * Thành phần `role="status"` (aria-live ngầm) nên trình đọc màn hình thông báo
 * đúng một lần; chi tiết trang trí (chấm) đều `aria-hidden`.
 */

/** Định dạng thời gian đã trôi: dưới một phút là giây, trên một phút là `phút:giây`. */
export function formatElapsed(totalSeconds: number): string {
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  return `${minutes}:${String(totalSeconds % 60).padStart(2, "0")}`;
}

/** Số giây đã trôi từ `startedAt`, cập nhật mỗi giây; `null` khi không có mốc giờ. */
function useElapsedSeconds(startedAt: number | null): number | null {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (startedAt === null) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [startedAt]);
  if (startedAt === null) return null;
  return Math.max(0, Math.floor((now - startedAt) / 1000));
}

export function ThinkingIndicator({
  startedAt,
  stopping = false,
}: {
  startedAt: number | null;
  /** Run đang được yêu cầu dừng: đổi nhãn thay vì để đồng hồ tiếp tục như thể vẫn đang nghĩ. */
  stopping?: boolean;
}) {
  const { t } = useI18n();
  const elapsed = useElapsedSeconds(startedAt);
  const dotsRef = useRef<HTMLSpanElement>(null);

  useGsapAnimate(() => {
    const element = dotsRef.current;
    if (!element) return;
    // Ba chấm nhảy nối tiếp nhau (stagger) rồi lặp vô hạn — dấu hiệu "vẫn đang
    // nghĩ" mềm hơn so với spinner quay. Đồng hồ tick bên cạnh là bằng chứng
    // thời gian thực; chấm chỉ giữ nhịp thị giác.
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
      {stopping ? t("chat.stopping") : t("chat.thinking")}
      {/* `hub-num` giữ bề rộng số ổn định để nhãn không giật khi giây nhảy. */}
      {elapsed !== null && elapsed > 0 ? (
        <span className="hub-num opacity-80">· {formatElapsed(elapsed)}</span>
      ) : null}
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
