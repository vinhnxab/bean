import { ChevronDownIcon } from "lucide-react";
import { useEffect, useState } from "react";

import type { DecisionDto } from "@/api/bindings";
import { useI18n } from "@/i18n";
import type { ConfirmState } from "@/store/chat";

function formatSeconds(seconds: number): string {
  const minutes = Math.floor(seconds / 60)
    .toString()
    .padStart(2, "0");
  const rest = (seconds % 60).toString().padStart(2, "0");
  return `${minutes}:${rest}`;
}

// (D27) Màu dòng diff trong thẻ xác nhận — mọi màu đi qua token CSS (mục 12.1).
// `+` = thêm (systemGreen), `-` = xoá (systemRed), header/hunk = mực mờ; dòng
// context (bắt đầu space) giữ màu chữ thường.
const DIFF_ADD_CLASS = "text-live";
const DIFF_DEL_CLASS = "text-alert";
const DIFF_META_CLASS = "text-ink-muted";

function diffLineClass(line: string): string | null {
  if (line.startsWith("--- a/") || line.startsWith("+++ b/")) return DIFF_META_CLASS;
  if (line.startsWith("@@")) return DIFF_META_CLASS;
  if (line.startsWith("+")) return DIFF_ADD_CLASS;
  if (line.startsWith("-")) return DIFF_DEL_CLASS;
  return null;
}

/**
 * Tìm vị trí bắt đầu khối diff trong prompt xác nhận: chỉ tin cậy cặp header
 * chuẩn `--- a/` + `+++ b/`. Trả -1 khi prompt không có diff (vd. `run_shell`)
 * — khi đó render nguyên văn như cũ, và phần mô tả phía trước header cũng
 * không bị tô màu oan (nội dung do người dùng/agent viết có thể bắt đầu `-`).
 */
function findDiffStart(lines: string[]): number {
  return lines.findIndex(
    (line, index) => line.startsWith("--- a/") && lines[index + 1]?.startsWith("+++ b/"),
  );
}

export function ConfirmCard({
  confirm,
  onDecision,
  onExpire,
}: {
  confirm: ConfirmState;
  onDecision: (decision: DecisionDto) => void;
  onExpire: (confirmId: string) => void;
}) {
  const { t } = useI18n();
  const [remaining, setRemaining] = useState(() =>
    Math.max(0, Math.ceil((confirm.receivedAt + confirm.timeout_seconds * 1000 - Date.now()) / 1000)),
  );
  // Thu gọn nội dung hành động: diff/lệnh dài chiếm nhiều chỗ trong luồng chat,
  // nhất là sau khi đã duyệt xong. `null` = chưa can thiệp tay — khi đó thẻ tự
  // thu gọn ngay khi được giải quyết, bấm tiêu đề để xem lại nguyên văn.
  const [manualCollapsed, setManualCollapsed] = useState<boolean | null>(null);

  useEffect(() => {
    const update = () => {
      const next = Math.max(
        0,
        Math.ceil((confirm.receivedAt + confirm.timeout_seconds * 1000 - Date.now()) / 1000),
      );
      setRemaining(next);
      if (next === 0 && confirm.resolution === "pending") onExpire(confirm.confirm_id);
    };
    update();
    const timer = window.setInterval(update, 1000);
    return () => window.clearInterval(timer);
  }, [confirm.confirm_id, confirm.receivedAt, confirm.resolution, confirm.timeout_seconds, onExpire]);

  const resolved =
    confirm.resolution === "allowed" || confirm.resolution === "denied" || confirm.resolution === "expired";
  const status =
    confirm.resolution === "submitting"
      ? t("confirm.submitting")
      : resolved
        ? confirm.resolution === "allowed"
          ? t("confirm.allowed")
          : confirm.resolution === "denied"
            ? t("confirm.denied")
            : t("confirm.expired")
        : `${t("confirm.expires")} ${formatSeconds(remaining)}`;
  // TINH TẾ: không viền, không shadow, không accent — chỉ một lớp wash rất nhạt
  // mang màu ngữ nghĩa; chấm màu ở badge lo phần còn lại. Thêm khung là quay
  // lại kiểu "thẻ SaaS" giữa luồng chat đã nhiều đường kẻ.
  const riskClass = confirm.risk === "dangerous" ? "bg-alert/8" : "bg-need/8";
  // Nhãn rủi ro hết lặng hơn pill nền đặc: chấm màu + chữ màu. Chữ trên nền
  // app dùng đúng token `-alert`/`-need` của từng chủ đề, không hardcode.
  const riskTextClass = confirm.risk === "dangerous" ? "text-alert" : "text-need";
  const riskDotClass = confirm.risk === "dangerous" ? "bg-alert" : "bg-need";
  const riskText = t(`confirm.risk.${confirm.risk}`);
  // Trạng thái sau khi giải quyết mang màu ngữ nghĩa: allowed = `--live` (xong),
  // denied/expired = mực mờ (đã đóng). Số đếm ngược dùng chữ số tabular để
  // không giật cột mỗi giây.
  const statusClass = confirm.resolution === "allowed" ? "text-live" : resolved ? "text-ink-muted" : "";
  const collapsed = manualCollapsed ?? resolved;
  const firstPromptLine = confirm.prompt.split("\n")[0] ?? "";

  function decide(decision: DecisionDto) {
    if (confirm.resolution !== "pending" || remaining === 0) return;
    onDecision(decision);
  }

  return (
    <section
      className={`glass-sheen rounded-lg p-4 ${riskClass}`}
      role="alertdialog"
      aria-labelledby={`confirm-${confirm.confirm_id}`}
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        {/* Nút thu gọn theo mẫu accordion (WAI-ARIA): nút nằm trong heading,
            `aria-expanded` báo trạng thái — không dùng phím tắt, không tự focus. */}
        <h2 id={`confirm-${confirm.confirm_id}`} className="flex items-center font-semibold">
          <button
            type="button"
            onClick={() => setManualCollapsed(!collapsed)}
            aria-expanded={!collapsed}
            title={collapsed ? t("confirm.expand") : t("confirm.collapse")}
            className="flex items-center gap-1 rounded text-left transition-colors hover:text-ink"
          >
            <ChevronDownIcon
              aria-hidden="true"
              className={`size-4 shrink-0 text-ink-muted transition-transform ${collapsed ? "-rotate-90" : ""}`}
            />
            {t("confirm.title")}
          </button>
        </h2>
        <span
          className={`inline-flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide ${riskTextClass}`}
        >
          <span aria-hidden="true" className={`size-1.5 rounded-full ${riskDotClass}`} />
          {riskText}
        </span>
      </div>
      {collapsed ? (
        // Dòng nhận diện gọn khi thu gọn: dòng đầu của prompt (lệnh/đường dẫn).
        // Nút quyết định và trạng thái vẫn hiện ở dưới — thẻ đang chờ thu gọn
        // bằng tay vẫn đủ để hành động.
        <p className="truncate font-mono text-xs text-ink-muted" title={firstPromptLine}>
          {firstPromptLine}
        </p>
      ) : (
        <>
          <p className="mb-1 text-xs font-medium text-ink-muted">{t("confirm.action")}</p>
          <pre className="whitespace-pre-wrap break-words rounded-md bg-surface-raised p-3 font-mono text-sm">
            {(() => {
              // (D27) Prompt có khối diff ⇒ tô màu từng dòng bằng <span> (React tự
              // escape, KHÔNG dangerouslySetInnerHTML — mục 12.3). Prompt thường
              // (run_shell…) giữ nguyên một text node. Key là offset ký tự của dòng
              // trong prompt — ổn định và duy nhất kể cả khi nội dung trùng nhau.
              const lines = confirm.prompt.split("\n");
              const diffStart = findDiffStart(lines);
              if (diffStart === -1) return confirm.prompt;
              let offset = 0;
              const items = lines.map((line) => {
                const item = { line, offset };
                offset += line.length + 1;
                return item;
              });
              return items.map((item, position) => {
                const kindClass = position >= diffStart ? diffLineClass(item.line) : null;
                const text = position < items.length - 1 ? `${item.line}\n` : item.line;
                return (
                  <span key={item.offset} className={kindClass ?? undefined}>
                    {text}
                  </span>
                );
              });
            })()}
          </pre>
        </>
      )}
      <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
        <span className={`hub-num text-sm font-medium ${statusClass}`} aria-live="polite">
          {status}
        </span>
        {!resolved ? (
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => decide("allow")}
              disabled={confirm.resolution === "submitting" || remaining === 0}
              className="rounded-md bg-brand px-3 py-2 text-sm font-medium text-brand-ink hover:bg-brand-strong disabled:opacity-50"
            >
              {t("confirm.allow")}
            </button>
            {confirm.allow_session_option && confirm.risk !== "dangerous" ? (
              <button
                type="button"
                onClick={() => decide("allow_in_session")}
                disabled={confirm.resolution === "submitting" || remaining === 0}
                className="rounded-md border border-brand px-3 py-2 text-sm font-medium text-brand hover:bg-brand-wash disabled:opacity-50"
              >
                {t("confirm.allowSession")}
              </button>
            ) : null}
            <button
              type="button"
              onClick={() => decide("deny")}
              disabled={confirm.resolution === "submitting" || remaining === 0}
              className="rounded-md border border-border px-3 py-2 text-sm font-medium hover:bg-surface-hover disabled:opacity-50"
            >
              {t("confirm.deny")}
            </button>
          </div>
        ) : null}
      </div>
    </section>
  );
}
