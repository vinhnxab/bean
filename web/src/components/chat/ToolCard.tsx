import gsap from "gsap";
import { useRef, useState } from "react";

import { api } from "@/api/client";
import type { StoredImage } from "@/features/chat/messages";
import { useI18n } from "@/i18n";
import { useGsapAnimate } from "@/lib/anim";

export type ToolCardStatus = "running" | "ok" | "error";

export function ToolCard({
  name,
  summary,
  argsPreview,
  outputPreview,
  status,
  messageId,
  image,
}: {
  name: string;
  summary: string;
  argsPreview: string;
  outputPreview: string;
  status: ToolCardStatus;
  messageId?: number;
  /** Ảnh chụp từ trang (M26 — `browser_screenshot`); `null` với tool khác. */
  image?: StoredImage | null;
}) {
  const { t } = useI18n();
  const [full, setFull] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [showFull, setShowFull] = useState(false);
  const cardRef = useRef<HTMLElement>(null);
  const badgeRef = useRef<HTMLSpanElement>(null);
  const previousStatus = useRef<ToolCardStatus>(status);

  // Chuyển từ "đang chạy" sang xong/lỗi là MỘT mốc tiến trình: badge bung vào
  // lại và thẻ chớp nhẹ độ sáng — mắt người dùng bắt được sự kiện mà không cần
  // đọc chữ. Không có `from`-tween ở đây vì thẻ đã có hoạt ảnh vào từ `<Enter>`.
  useGsapAnimate(() => {
    const from = previousStatus.current;
    previousStatus.current = status;
    if (from === status || from !== "running") return;
    const badge = badgeRef.current;
    if (badge)
      gsap.fromTo(
        badge,
        { opacity: 0, scale: 0.5 },
        { opacity: 1, scale: 1, duration: 0.45, ease: "back.out(2.5)", clearProps: "opacity,transform" },
      );
    const card = cardRef.current;
    if (card)
      gsap.fromTo(
        card,
        { filter: "brightness(1.9)" },
        { filter: "brightness(1)", duration: 0.55, ease: "power2.out", clearProps: "filter" },
      );
  }, [status]);

  const statusText =
    status === "running" ? t("tool.running") : status === "ok" ? t("tool.ok") : t("tool.error");

  async function openFull() {
    if (!messageId || full !== null) {
      setShowFull(true);
      return;
    }
    setLoading(true);
    try {
      const dto = await api.getMessage(messageId);
      const message = dto.message;
      setFull(
        typeof message === "object" &&
          message !== null &&
          !Array.isArray(message) &&
          typeof message.text === "string"
          ? message.text
          : "",
      );
    } catch {
      setFull(t("chat.historyError"));
    } finally {
      setLoading(false);
      setShowFull(true);
    }
  }

  return (
    <article
      ref={cardRef}
      className="rounded-lg border border-border bg-card p-3 shadow-sm"
      aria-label={name}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <h3 className="truncate font-mono text-sm font-semibold">{name}</h3>
          <p className="truncate text-sm text-muted-foreground">{summary}</p>
        </div>
        <span
          ref={badgeRef}
          className={`rounded-full px-2 py-1 text-xs font-semibold ${status === "error" ? "bg-destructive/10 text-destructive" : status === "ok" ? "bg-live/10 text-live" : "bg-need/10 text-need"}`}
        >
          {statusText}
        </span>
      </div>
      <details className="mt-3">
        <summary className="cursor-pointer text-sm font-medium">{t("tool.args")}</summary>
        <pre className="mt-2 whitespace-pre-wrap break-words rounded-lg bg-accent p-3 font-mono text-xs">
          {argsPreview}
        </pre>
      </details>
      <div className="mt-3">
        <p className="mb-1 text-sm font-medium">{t("tool.output")}</p>
        {image ? (
          // Ảnh chụp từ trang do model kiểm soát, nên nó là nội dung KHÔNG TIN
          // CẬY. Vẫn hiển thị bằng thẻ `<img>` — nhưng `src` chỉ có thể là
          // `data:` URI (đã kiệm ở `parseImage`), không bao giờ là URL ngoài; đây là
          // điều CSP `img-src 'self' data:` chốt lại thành lớp thứ hai. Không
          // dùng `dangerouslySetInnerHTML` ở đây.
          <img
            src={`data:${image.mediaType};base64,${image.data}`}
            alt={outputPreview || name}
            className="max-h-96 w-full rounded-lg border border-border object-contain"
          />
        ) : null}
        <pre
          className={`whitespace-pre-wrap break-words rounded-lg bg-card p-3 font-mono text-xs leading-5 text-foreground ${image ? "mt-2" : ""}`}
        >
          {outputPreview || t("tool.noOutput")}
        </pre>
      </div>
      {messageId ? (
        <button
          type="button"
          onClick={openFull}
          disabled={loading}
          className="mt-2 text-sm font-semibold text-live hover:underline disabled:opacity-50"
        >
          {loading ? t("common.loading") : t("tool.viewFull")}
        </button>
      ) : null}
      {showFull ? (
        <div
          className="mt-3 rounded-lg border border-border p-3"
          role="dialog"
          aria-label={t("tool.fullTitle")}
        >
          <div className="mb-2 flex justify-end">
            <button
              type="button"
              onClick={() => setShowFull(false)}
              className="text-sm text-muted-foreground hover:text-foreground"
            >
              {t("common.close")}
            </button>
          </div>
          <pre className="whitespace-pre-wrap break-words font-mono text-xs">{full ?? ""}</pre>
        </div>
      ) : null}
    </article>
  );
}
