import type { DecisionDto } from "@/api/bindings";
import { Button } from "@/components/ui/button";
import { useRealtime } from "@/features/chat/RealtimeProvider";
import { useI18n } from "@/i18n";
import type { ConfirmState } from "@/store/chat";
import { useChatStore } from "@/store/chat";

/**
 * Hàng chờ duyệt — vị trí có trọng số thị giác cao nhất sau sơ đồ, vì đây là hành
 * động thật sự cần làm ngay khi mở app.
 *
 * Đọc thẳng từ `useChatStore`, tức là từ `Sync` đã được server lọc RBAC — không mở
 * thêm đường đọc nào, đúng ràng buộc "HUB đọc qua WS Sync/REST hiện có".
 */
export function ConfirmQueue() {
  const { t } = useI18n();
  const { sendConfirm } = useRealtime();
  const confirmsById = useChatStore((state) => state.confirmsById);
  const pending = Object.values(confirmsById).filter((item) => item.resolution === "pending");

  // Rỗng ⇒ không render gì: trạng thái rỗng ở đây là "không có việc gì", và một
  // khung trống chỉ làm loãng thứ đáng chú ý nhất màn hình.
  if (pending.length === 0) return null;

  return (
    <section
      aria-labelledby="hub-confirm-title"
      data-testid="hub-confirm-queue"
      className="rounded-md border border-need bg-surface-raised p-4 sm:p-5"
    >
      <h2 id="hub-confirm-title" className="text-sm font-semibold text-need">
        {t("hub.confirm.title", { count: String(pending.length) })}
      </h2>
      <ul className="mt-3 flex flex-col gap-2">
        {pending.map((confirm) => (
          <ConfirmRow
            key={confirm.confirm_id}
            confirm={confirm}
            onDecision={(decision) => sendConfirm(confirm.confirm_id, decision)}
          />
        ))}
      </ul>
    </section>
  );
}

function ConfirmRow({
  confirm,
  onDecision,
}: {
  confirm: ConfirmState;
  onDecision: (decision: DecisionDto) => void;
}) {
  const { t } = useI18n();
  const decide = (decision: DecisionDto) => onDecision(decision);
  return (
    <li className="flex flex-col gap-2 border-t border-rule pt-2 first:border-t-0 first:pt-0 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
      <div className="min-w-0">
        {/* Nguyên văn hành động dùng monospace: đây là thứ máy sẽ chạy, người dùng
            phải đọc CHÍNH XÁC chứ không phải đọc "ý nghĩa" của nó. */}
        <p className="break-words font-mono text-xs">{confirm.prompt}</p>
        <p className="mt-1 text-xs text-ink-muted">
          {t(`confirm.risk.${confirm.risk}`)}
          {confirm.role ? ` · ${confirm.role}` : ""}
        </p>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        {/* "Cho phép trong phiên" chỉ hiện khi server nói được — Dangerous thì không
            có tuỳ chọn này (agents.md mục 7.2), nên không render nút "chết". */}
        {confirm.allow_session_option ? (
          <Button variant="outline" size="sm" onClick={() => decide("allow_in_session")}>
            {t("confirm.allowSession")}
          </Button>
        ) : null}
        <Button variant="attention" size="sm" onClick={() => decide("allow")}>
          {t("confirm.allow")}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => decide("deny")}>
          {t("confirm.deny")}
        </Button>
      </div>
    </li>
  );
}
