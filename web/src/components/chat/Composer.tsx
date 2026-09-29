import { ArrowUpIcon, SquareIcon } from "lucide-react";
import { useLayoutEffect, useRef } from "react";

import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";

/** Chiều cao tối đa trước khi khung bắt đầu cuộn bên trong. */
const MAX_HEIGHT_PX = 200;

/**
 * Ô soạn tin nhắn: tự giãn theo nội dung, nút gửi tròn bám góc phải.
 *
 * # Vì sao tự giãn thay vì `resize-y`
 *
 * Ở bản cũ textarea cao cố định 2 dòng và có tay nắm kéo. Ba vấn đề: (1) kéo
 * tay nắm bằng chuột trên điện thoại gần như không khả thi; (2) ô cao sẵn
 * chiếm màn hình dù chỉ có một dòng; (3) nút "Gửi" viết chữ nằm ngoài ô, xê
 * dịch theo chiều cao ô nên nhảy nhót mỗi lần gõ dòng thứ ba.
 *
 * Tự giãn theo `scrollHeight` giữ nút gửi **đứng yên ở đáy khung** và luôn nhìn
 * thấy kể cả khi đã cuộn hết vùng soạn.
 */
export type ComposerProps = {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  onStop: () => void;
  /** Đang chạy → hiện nút Dừng thay cho nút Gửi. */
  canStop: boolean;
  stopping: boolean;
  disabled: boolean;
  sendError: boolean;
};

export function Composer({
  value,
  onChange,
  onSubmit,
  onStop,
  canStop,
  stopping,
  disabled,
  sendError,
}: ComposerProps) {
  const { t } = useI18n();
  const ref = useRef<HTMLTextAreaElement>(null);

  /**
   * Tự giãn theo nội dung.
   *
   * Vì sao đọc `value` **bên trong** effect thay vì để dependency: hiệu ứng cần
   * chạy lại đúng khi `value` đổi, nhưng lại không dùng `value` để tính gì —
   * việc đo dựa trên `scrollHeight` của chính DOM. Khai báo `[value]` sẽ bị
   * `useExhaustiveDependencies` báo thừa; callback nội tuyến giữ đúng ý nghĩa
   * "đo lại khi nội dung đổi" mà không khai báo sai.
   *
   * Đặt lại `height = "auto"` trước khi đo là bắt buộc: nếu không, textarea giữ
   * chiều cao cũ và **không bao giờ co lại** khi người dùng xoá nhiều dòng.
   *
   * `useLayoutEffect` chứ không `useEffect`: phải căn chiều cao trước khi
   * trình duyệt vẽ, nếu không khung soạn nhấp nháy một nhịp mỗi lần gõ.
   *
   * `biome-ignore` phải nằm ngay dòng trên hook — comment nhiều dòng ở giữa
   * làm chỉ thị này không áp dụng (và lỗi lint quay lại).
   */
  // biome-ignore lint/correctness/useExhaustiveDependencies: `value` là điều kiện kích hoạt có chủ đích — đo lại khi nội dung đổi, dù thân effect đo bằng `scrollHeight` chứ không đọc `value`. Bỏ nó sẽ làm ô soạn chỉ tự giãn một lần rồi đứng yên.
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, MAX_HEIGHT_PX)}px`;
  }, [value]);

  const canSend = !disabled && value.trim().length > 0;

  return (
    // Gradient mờ dần thay cho `border-t` cứng: vùng soạn nổi lên khỏi dòng
    // thời gian mà không cắt ngang danh sách bằng một đường kẻ.
    <div className="bg-gradient-to-t from-surface via-surface to-transparent px-3 pb-3 pt-2 md:px-6">
      <form
        className="mx-auto max-w-4xl"
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
      >
        <div
          className={cn(
            "flex items-end gap-2 rounded-xl border bg-surface-raised p-2 transition-colors",
            "focus-within:border-brand",
            sendError ? "border-alert" : "border-rule",
          )}
        >
          <textarea
            ref={ref}
            value={value}
            onChange={(event) => onChange(event.target.value)}
            onKeyDown={(event) => {
              // `isComposing`: IME tiếng Việt — Enter đang chọn dấu, gửi tin ở
              // giữa lúc gõ là mất dấu. Bỏ qua Enter khi đang compose.
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                onSubmit();
              }
            }}
            rows={1}
            placeholder={t("chat.placeholder")}
            disabled={disabled}
            aria-label={t("chat.placeholder")}
            className="max-h-[200px] min-h-10 flex-1 resize-none bg-transparent px-2 py-2 text-sm leading-6 outline-none placeholder:text-muted-foreground"
          />
          {canStop ? (
            <button
              type="button"
              onClick={onStop}
              disabled={stopping}
              aria-label={stopping ? t("chat.stopping") : t("chat.stop")}
              title={stopping ? t("chat.stopping") : t("chat.stop")}
              className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-alert text-alert transition-colors hover:bg-alert hover:text-alert-ink disabled:opacity-50"
            >
              <SquareIcon className="size-3.5 fill-current" />
            </button>
          ) : (
            <button
              type="submit"
              disabled={!canSend}
              aria-label={t("chat.send")}
              title={t("chat.send")}
              // Nút tròn 32px đúng như Open WebUI: thao tác gửi lặp lại liên tục
              // nên phải là mục tiêu chạm lớn và không cần đọc nhãn.
              className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand text-brand-ink transition-colors hover:bg-brand-strong disabled:pointer-events-none disabled:opacity-40"
            >
              <ArrowUpIcon className="size-4" />
            </button>
          )}
        </div>
        {sendError ? (
          <p className="mt-2 text-sm text-alert" role="alert">
            {t("chat.sendError")}
          </p>
        ) : (
          // Gợi ý phím tắt ở độ mờ thấp, chỉ đọc được khi người dùng chủ động
          // tìm — không phải dòng chữ cạnh tranh với lời cuội.
          <p className="mt-1.5 text-right text-xs text-muted-foreground/70">{t("chat.enterHint")}</p>
        )}
      </form>
    </div>
  );
}
