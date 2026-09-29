import { CheckIcon, CopyIcon } from "lucide-react";
import { isValidElement, type ReactNode, useState } from "react";

import { BeanAvatar } from "@/components/brand/BeanAvatar";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";

/**
 * Một tin nhắn trong dòng thời gian, trải hết chiều ngang — không bọc bubble.
 *
 * # Vì sao bỏ bubble
 *
 * Bean hiển thị markdown dài, khối code và thẻ tool. Bubble ép nội dung vào
 * `max-w-[85%]` nên khối code phải cuộn ngang vô nghĩa và cột đọc bị hẹp lại
 * giữa lúc người dùng đang tập trung đọc. Open WebUI cũng mặc định phẳng
 * (`chatBubble` mặc định tắt) vì lý do đó.
 *
 * Vẫn còn dấu hiệu của bubble: tin của **bạn** căn phải và hẹp ở `85%`. Đó là
 * chủ ý — hai bên phải khác nhau để người đọc lướt dòng thời gian không phải
 * suy nghĩ "câu này của ai"; phía Bean giữ nguyên chiều rộng vì đó là bên
 * mang nội dung dài.
 *
 * # Vì sao thanh thao tác chỉ hiện khi rê
 *
 * Sao chép là hành động hiếm; để nó sáng suốt cạnh nội dung sẽ cạnh tranh chú
 * ý với chính câu trả lời. Nhưng **bàn phím không có hover**, nên thanh thao
 * tác còn hiện khi focus trong vùng — nếu không, người dùng bàn phím không có
 * đường tới nút sao chép.
 */

export type MessageRowProps = {
  /**
   * Ai gửi tin: `user` (bạn, căn phải) hay `assistant` (Bean, căn trái).
   *
   * Cố tình **không** đặt tên prop là `role`: `role` là thuộc tính ARIA với
   * tập giá trị riêng, và dùng nó cho "ai gửi" làm Biome báo
   * `useValidAriaRole` (giá trị `user`/`assistant` không phải role ARIA hợp lệ)
   * — lỗi đúng, vì đây đúng là nhầm lẫn khái niệm.
   */
  author: "user" | "assistant";
  children: ReactNode;
  /** Giờ ghi, hiển thị gọn bên cạnh tên. */
  timestamp?: string | null;
  /** Dùng cho tiêu đề `aria-labelledby` khi tin là của Bean. */
  headingId?: string;
};

export function MessageRow({ author, children, timestamp, headingId }: MessageRowProps) {
  const { t, lang } = useI18n();
  const [copied, setCopied] = useState(false);
  const isUser = author === "user";
  const label = isUser ? t("message.you") : t("hub.manager.name");

  // `aria-labelledby` trỏ tới phần tử chứa tên người gửi: đọc bằng màn hình
  // đọc sẽ nghe "Bạn, 14:32" rồi tới nội dung, thay vì đọc nội dung trần trụi.
  return (
    <article
      // `group` mở thanh thao tác khi rê; `focus-within` mở khi dùng bàn phím.
      className="message-row group rounded-lg px-2 py-2 transition-colors hover:bg-surface-raised/40 focus-within:bg-surface-raised/40 md:px-3"
      aria-labelledby={headingId}
    >
      <div className={cn("flex gap-3", isUser && "flex-row-reverse")}>
        <BeanAvatar size={28} className="mt-0.5" title={label} />
        <div className={cn("min-w-0 flex-1", isUser && "flex flex-col items-end")}>
          <div
            id={headingId}
            className={cn(
              "flex items-center gap-2 text-xs text-muted-foreground",
              // Ở tin của bạn, avatar đã ở bên phải nên tên cũng phải phải để
              // nhãn nằm cùng phía với avatar, nếu không thành hai cụm lệch nhau.
              isUser && "flex-row-reverse",
            )}
          >
            <span className={cn("font-medium", isUser ? "text-ink" : "text-brand")}>{label}</span>
            {timestamp ? (
              <time className="hub-num" dateTime={timestamp}>
                {formatTime(timestamp, lang)}
              </time>
            ) : null}
          </div>
          <div className={cn("mt-1 min-w-0", isUser ? "max-w-[85%]" : "w-full")}>{children}</div>
        </div>
      </div>
      <div
        className={cn(
          "mt-1 flex gap-1",
          // Ẩn mặc định, hiện khi rê chuột hoặc khi focus bằng bàn phím.
          "opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100",
          isUser ? "justify-end pr-10" : "pl-10",
        )}
      >
        <CopyButton value={textOf(children)} copied={copied} onCopied={setCopied} />
      </div>
    </article>
  );
}

function CopyButton({
  value,
  copied,
  onCopied,
}: {
  value: string;
  copied: boolean;
  onCopied: (next: boolean) => void;
}) {
  const { t } = useI18n();
  // Nút vô hiệu hoá thay vì ẩn: người dùng bàn phím tab tới đây cần biết là có
  // nút nhưng không dùng được, hơn là nút biến mất giữa chừng.
  const empty = value.trim().length === 0;
  return (
    <button
      type="button"
      disabled={empty}
      onClick={() => {
        void navigator.clipboard?.writeText(value).then(
          () => {
            onCopied(true);
            window.setTimeout(() => onCopied(false), 1400);
          },
          () => onCopied(false),
        );
      }}
      aria-label={t("message.copy")}
      title={copied ? t("message.copied") : t("message.copy")}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs transition-colors",
        "text-muted-foreground hover:bg-accent hover:text-ink disabled:pointer-events-none disabled:opacity-40",
      )}
    >
      {copied ? <CheckIcon className="size-3.5 text-live" /> : <CopyIcon className="size-3.5" />}
      <span>{copied ? t("common.copied") : t("common.copy")}</span>
    </button>
  );
}

function formatTime(iso: string, lang: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  // Chỉ giờ, không ngày: cột thời gian đã có nhóm ngày ở sidebar và dải ngày
  // trong khung chat, lặp lại cả ngày ở mỗi tin chỉ làm rối.
  return new Intl.DateTimeFormat(lang, { timeStyle: "short" }).format(date);
}

/** Bóc text thuần từ cây React để nút sao chép không cần biết nội dung render. */
function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children);
  return "";
}
