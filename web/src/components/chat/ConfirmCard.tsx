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
  const riskClass =
    confirm.risk === "dangerous" ? "border-destructive/40 bg-destructive/10" : "border-need bg-need/10";
  const riskText = t(`confirm.risk.${confirm.risk}`);

  function decide(decision: DecisionDto) {
    if (confirm.resolution !== "pending" || remaining === 0) return;
    onDecision(decision);
  }

  return (
    <section
      className={`rounded-lg border p-4 shadow-sm ${riskClass}`}
      role="alertdialog"
      aria-labelledby={`confirm-${confirm.confirm_id}`}
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <h2 id={`confirm-${confirm.confirm_id}`} className="font-semibold">
          {t("confirm.title")}
        </h2>
        <span className="rounded-full bg-white/70 px-2 py-1 text-xs font-bold uppercase tracking-wide">
          {riskText}
        </span>
      </div>
      <p className="mb-1 text-xs font-medium opacity-70">{t("confirm.action")}</p>
      <pre className="whitespace-pre-wrap break-words rounded-lg bg-white/70 p-3 font-mono text-sm">
        {confirm.prompt}
      </pre>
      <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
        <span className="text-sm font-medium" aria-live="polite">
          {status}
        </span>
        {!resolved ? (
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => decide("allow")}
              disabled={confirm.resolution === "submitting" || remaining === 0}
              className="rounded-lg bg-brand px-3 py-2 text-sm font-semibold text-brand-ink hover:bg-brand-strong disabled:opacity-50"
            >
              {t("confirm.allow")}
            </button>
            {confirm.allow_session_option && confirm.risk !== "dangerous" ? (
              <button
                type="button"
                onClick={() => decide("allow_in_session")}
                disabled={confirm.resolution === "submitting" || remaining === 0}
                className="rounded-lg border border-brand px-3 py-2 text-sm font-semibold text-brand hover:bg-brand-wash disabled:opacity-50"
              >
                {t("confirm.allowSession")}
              </button>
            ) : null}
            <button
              type="button"
              onClick={() => decide("deny")}
              disabled={confirm.resolution === "submitting" || remaining === 0}
              className="rounded-lg border border-border px-3 py-2 text-sm font-semibold hover:bg-white/60 disabled:opacity-50"
            >
              {t("confirm.deny")}
            </button>
          </div>
        ) : null}
      </div>
    </section>
  );
}
