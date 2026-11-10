import { CheckIcon, CircleIcon, LoaderCircleIcon } from "lucide-react";

import type { PlanStepDto, PlanStepStatusDto } from "@/api/bindings";
import { useI18n } from "@/i18n";

/**
 * Thẻ kế hoạch bước của run đang chạy (D25).
 *
 * Model cập nhật kế hoạch qua tool `update_plan`; client nhận snapshot đầy đủ qua
 * event `plan` (hoặc `Sync` khi nối lại). Thẻ chỉ hiển thị khi run đang chạy —
 * kế hoạch là trạng thái sống theo run, không phải một tin nhắn trong lịch sử.
 *
 * # Màu trạng thái tách khỏi thương hiệu
 *
 * `done` dùng `--live` (xong), `in_progress` dùng `--brand` (hành động đang chạy),
 * `pending` dùng mực mờ — cùng hệ status của badge confirm, không hardcode màu.
 */
export function PlanCard({ steps }: { steps: PlanStepDto[] }) {
  const { t } = useI18n();
  if (steps.length === 0) return null;
  const doneCount = steps.filter((step) => step.status === "done").length;
  return (
    <section className="glass-sheen rounded-lg bg-surface-raised/40 p-4" aria-label={t("chat.plan.title")}>
      <div className="mb-2 flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold">{t("chat.plan.title")}</h2>
        <span className="hub-num text-xs text-ink-muted">
          {t("chat.plan.progress", { done: doneCount, total: steps.length })}
        </span>
      </div>
      <ol className="space-y-1.5">
        {steps.map((step) => (
          <li key={`${step.status}:${step.title}`} className="flex items-start gap-2 text-sm">
            <PlanStepIcon status={step.status} />
            <span className={step.status === "done" ? "text-ink-muted line-through" : ""}>{step.title}</span>
          </li>
        ))}
      </ol>
    </section>
  );
}

function PlanStepIcon({ status }: { status: PlanStepStatusDto }) {
  if (status === "done") {
    return (
      <span
        className="mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full bg-live/15"
        aria-hidden="true"
      >
        <CheckIcon className="size-3 text-live" />
      </span>
    );
  }
  if (status === "in_progress") {
    return (
      <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center" aria-hidden="true">
        <LoaderCircleIcon className="size-4 animate-spin text-brand" />
      </span>
    );
  }
  return (
    <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center" aria-hidden="true">
      <CircleIcon className="size-3 text-ink-muted opacity-50" />
    </span>
  );
}
