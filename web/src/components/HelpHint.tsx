import { CircleHelp } from "lucide-react";
import { useId } from "react";

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";

/**
 * HelpHint — pattern "dấu hỏi" cho description: thay vì chen một đoạn chữ giải
 * thích dài dưới tiêu đề, tiêu đề chỉ kèm một nút `?`; bấm vào thì nội dung
 * giải thích hiện ra trong popover **kế bên nút** (không phải giữa màn hình).
 *
 * # Tham khảo
 *
 * - shadcn/ui Popover (ui.shadcn.com/docs/components/popover) — cấu trúc
 *   `Popover > PopoverTrigger + PopoverContent`, nền `@radix-ui/react-popover`.
 *   Project dùng đúng hệ shadcn/ui + Radix nên pattern này cắm vào tự nhiên.
 * - Cùng họ với help-bubble trên GitHub/Linear: icon nhỏ, giải thích theo ngữ
 *   cảnh, không chặn phần còn lại của trang.
 *
 * # A11y
 *
 * Trigger là `button` thật (Radix tự đặt `aria-expanded`, `aria-haspopup`,
 * `aria-controls`, đóng bằng Escape, trả focus về trigger khi đóng). Nội dung
 * popover được nối `aria-labelledby` tới tiêu đề giải thích qua `useId()` — an
 * toàn khi trang có nhiều HelpHint. Icon chỉ là trang trí, `aria-hidden`.
 */
export function HelpHint({
  title,
  text,
  testId,
  label,
  className,
}: {
  title: string;
  text: string;
  testId?: string;
  /** Ghi đè nhãn đọc màn hình (mặc định `common.helpHint`) — dùng khi nút đứng cạnh
   *  tiêu đề chung chung và cần nói rõ giải thích thứ gì. */
  label?: string;
  /** Nới thêm/khoá class cho trigger, ví dụ thu nhỏ trong node dày đặc. */
  className?: string;
}) {
  const { t } = useI18n();
  const titleId = useId();

  return (
    <Popover>
      <PopoverTrigger
        type="button"
        aria-label={label ?? t("common.helpHint")}
        data-testid={testId ?? "help-hint"}
        className={cn(
          "inline-flex size-6 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/40 focus-visible:outline-none",
          className,
        )}
      >
        <CircleHelp className="size-4" aria-hidden="true" />
      </PopoverTrigger>
      <PopoverContent aria-labelledby={titleId}>
        <p id={titleId} className="text-sm font-semibold">
          {title}
        </p>
        <p className="mt-1 text-sm text-muted-foreground">{text}</p>
      </PopoverContent>
    </Popover>
  );
}
