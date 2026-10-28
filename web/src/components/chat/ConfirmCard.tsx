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
        <h2 id={`confirm-${confirm.confirm_id}`} className="font-semibold">
          {t("confirm.title")}
        </h2>
        <span
          className={`inline-flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide ${riskTextClass}`}
        >
          <span aria-hidden="true" className={`size-1.5 rounded-full ${riskDotClass}`} />
          {riskText}
        </span>
      </div>
      <p className="mb-1 text-xs font-medium text-ink-muted">{t("confirm.action")}</p>
      <pre className="whitespace-pre-wrap break-words rounded-md bg-surface-raised p-3 font-mono text-sm">
        {confirm.prompt}
      </pre>
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
