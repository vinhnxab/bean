import gsap from "gsap";
import { type DependencyList, type ReactNode, useEffect, useRef } from "react";

import { cn } from "@/lib/utils";

/**
 * Hoạt ảnh cho Bean, dựng trên **GSAP** (gói npm, bundle cùng app — không CDN,
 * agents.md mục 0.9).
 *
 * Ba nguyên tắc:
 *
 * 1. **`prefers-reduced-motion` được tôn trọng.** Mọi hiệu ứng đi qua
 *    `useGsapAnimate`, vốn chạy bên trong
 *    `gsap.matchMedia("(prefers-reduced-motion: no-preference)")`: hệ thống bật
 *    "giảm chuyển động" thì không tween nào được tạo và phần tử hiện nguyên
 *    trạng thái cuối — không bao giờ bị kẹt `opacity: 0` (khác với đặt
 *    `gsap.from` ngoài điều kiện). Lật cài đặt giữa phiên cũng có hiệu lực ngay:
 *    GSAP tự chạy lại/dọn context khi điều kiện đổi.
 * 2. **Animation chỉ là trang trí.** Nội dung, nhãn, số đếm đều đọc được kể cả
 *    khi animation không chạy (jsdom trong test, trình duyệt thiếu API).
 * 3. **Không cascade lịch sử khi mở phiên.** Tải sẵn N tin mà animate cả N là
 *    một màn trượt hỗn loạn — đó là lý do `<Enter>` nhận `when` và chỉ chốt
 *    quyết định tại lần chạy effect đầu tiên của chính nó.
 */

/** Chỉ chạy khi người dùng KHÔNG bật "giảm chuyển động". */
const NO_REDUCTION = "(prefers-reduced-motion: no-preference)";

/** Thiếu `matchMedia` (môi trường hiếm, jsdom không polyfill) ⇒ coi như không có hoạt ảnh. */
function motionSupported(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function";
}

/** Hỏi trực tiếp hệ thống xem người dùng có bật "giảm chuyển động" không. */
export function prefersReducedMotion(): boolean {
  return motionSupported() && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * Chạy `setup` (tạo tween) trong một context `gsap.matchMedia` chỉ kích hoạt khi
 * người dùng không bật "giảm chuyển động". Mọi tween sinh ra trong `setup` được
 * GSAP gắn vào context: unmount hoặc lật điều kiện sẽ `revert()` — kill tween và
 * khôi phục style gốc, không để lại inline style mồ côi.
 *
 * `deps` do người gọi quyết định (giống mẫu trong `Composer`): callback thường
 * là closure inline nên phải tự khai báo đúng dependency.
 */
export function useGsapAnimate(setup: () => void, deps: DependencyList): void {
  useEffect(() => {
    if (!motionSupported()) return undefined;
    const mm = gsap.matchMedia();
    mm.add(NO_REDUCTION, () => {
      setup();
    });
    return () => mm.revert();
    // biome-ignore lint/correctness/useExhaustiveDependencies: helper nhận deps từ người gọi; `setup` là tham số tĩnh của chính hook này (mẫu tương tự useLayoutEffect trong Composer).
  }, deps);
}

/** Kiểu kết quả tween mà ChatPage cần giữ lại để huỷ khi có lượt cuộn mới. */
export type ScrollTween = { kill: () => void; isActive: () => boolean };

/**
 * Cuộn một container về đáy bằng tween thay vì nhảy tức thì.
 *
 * Trả về tween để người gọi `kill()` khi có nội dung mới hoặc người dùng can
 * thiệp (wheel); trả `null` khi ở chế độ giảm chuyển động (nhảy thẳng, đúng
 * hành vi máy đọc màn hình mong đợi).
 */
export function tweenToBottom(element: HTMLElement, duration = 0.45): ScrollTween | null {
  const target = element.scrollHeight - element.clientHeight;
  if (prefersReducedMotion()) {
    element.scrollTop = target;
    return null;
  }
  // Tween trên proxy rồi ghi lại `scrollTop`: giá trị lẻ này không phụ thuộc
  // việc GSAP có coi `scrollTop` là thuộc tính đặc biệt của DOM hay không.
  const proxy = { top: element.scrollTop };
  return gsap.to(proxy, {
    top: target,
    duration,
    ease: "power2.out",
    overwrite: true,
    onUpdate: () => {
      element.scrollTop = proxy.top;
    },
    // Nội dung dài thêm trong lúc đang trượt (delta streaming tới trễ): kết thúc
    // bằng một cú bám đáy cuối để không bị hụt nội dung tới sự kiện kế tiếp.
    onComplete: () => {
      element.scrollTop = element.scrollHeight;
    },
  });
}

export type EnterKind = "rise" | "pop" | "shake";

export type EnterProps = {
  /**
   * Có animate lúc mount không. Lịch sử tải sẵn truyền `false`; tin xuất hiện
   * sau (msg mới, tool, trả lời cuối) truyền `true`.
   */
  when?: boolean;
  /** Kiểu vào: trượt lên (mặc định), bung ra (thẻ/ pill), rung cảnh báo (lỗi). */
  kind?: EnterKind;
  className?: string;
  children: ReactNode;
};

/**
 * Bọc một phần tử để nó có hoạt ảnh **một lần khi mount**.
 *
 * `when` được chốt tại lần effect đầu tiên của instance: prop có đổi sau này
 * (store cập nhật làm `when` của một tin cũ trở thành `true`) cũng không kích
 * hoạt lại — nếu không, mở một phiên cũ sẽ trượt toàn bộ lịch sử ngay lần render
 * kế tiếp.
 */
export function Enter({ when = true, kind = "rise", className, children }: EnterProps) {
  const ref = useRef<HTMLDivElement>(null);
  const decided = useRef(false);

  useGsapAnimate(() => {
    const element = ref.current;
    if (!element || decided.current) return;
    decided.current = true;
    if (!when) return;
    if (kind === "pop") {
      gsap.from(element, {
        opacity: 0,
        scale: 0.85,
        duration: 0.34,
        ease: "back.out(1.8)",
        clearProps: "opacity,transform",
      });
    } else if (kind === "shake") {
      gsap.from(element, {
        opacity: 0,
        x: -14,
        duration: 0.7,
        ease: "elastic.out(1, 0.4)",
        clearProps: "opacity,transform",
      });
    } else {
      gsap.from(element, {
        opacity: 0,
        y: 14,
        duration: 0.4,
        ease: "power2.out",
        clearProps: "opacity,transform",
      });
    }
  }, [when, kind]);

  return (
    <div ref={ref} className={cn(className)}>
      {children}
    </div>
  );
}

export type StaggerProps = {
  /**
   * Selector chọn item cần animate trong container (vd `"li"`). Bỏ trống thì
   * animate các con trực tiếp.
   */
  item?: string;
  /** Delay trước khi bắt đầu chuỗi (để phần tử khác vào trước). */
  delay?: number;
  className?: string;
  children: ReactNode;
};

/**
 * Cho các phần tử trong một nhóm LỚN vào lần lượt — hiệu ứng "dữ liệu vừa về"
 * cho danh sách management (sidebar phiên, feed hoạt động, khối trang).
 *
 * Chạy **đúng một lần khi mount**: item thêm sau đó hiện ngay, không animate —
 * refetch dữ liệu không được re-trigger chuỗi (mỗi lần lọc/search là một trận
 * nhấp nháy). Thời gian dồn stagger bị cap ~0.45s: một list 30 hàng không được
 * biến thành hai giây chờ đợi, phần đuôi vào gần như cùng lúc.
 */
export function Stagger({ item, delay = 0, className, children }: StaggerProps) {
  const ref = useRef<HTMLDivElement>(null);

  useGsapAnimate(() => {
    const element = ref.current;
    if (!element) return;
    const targets = item ? Array.from(element.querySelectorAll(item)) : Array.from(element.children);
    if (targets.length === 0) return;
    gsap.from(targets, {
      opacity: 0,
      y: 10,
      duration: 0.4,
      ease: "power2.out",
      delay,
      stagger: Math.min(0.06, 0.45 / targets.length),
      clearProps: "opacity,transform",
    });
  }, [item, delay]);

  return (
    <div ref={ref} className={className}>
      {children}
    </div>
  );
}
