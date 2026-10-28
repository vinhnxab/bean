import { PlusIcon, XIcon } from "lucide-react";
import { SessionList } from "@/features/chat/SessionList";
import type { groupSessions } from "@/features/chat/sessionGroups";
import type { useSessions } from "@/features/sessions/queries";
import { useI18n } from "@/i18n";

/**
 * Sidebar phải — nơi duy nhất chứa các việc liên quan tới **hội thoại**:
 * tạo mới, tìm, xem đã lưu trữ, đổi hội thoại. Trước đây các thành phần này
 * nằm ở sidebar trái, trộn lẫn với điều hướng chính.
 *
 * # Hai trạng thái, một component
 *
 * - Desktop (`md:` trở lên): cột thường trú bên phải, **luôn mở** — không có
 *   nút đóng. `open` không có tác dụng vì `md:static md:translate-x-0` thắng.
 * - Mobile (`< md`): ngăn kéo trượt từ mép phải, phủ overlay mờ; điều khiển
 *   bằng `open`/`onClose`, nút mở là icon `PanelRight` trên `TopNav`. Cùng một
 *   pattern với ngăn kéo điều hướng bên trái.
 */
export function RightSidebar({
  open,
  onClose,
  onNavigate,
  onDeleted,
  onNewSession,
  creating,
  actionError,
  ...listProps
}: {
  open: boolean;
  onClose: () => void;
  onNavigate: () => void;
  onDeleted: (id: number) => void;
  onNewSession: () => void;
  creating: boolean;
  actionError: boolean;
  search: string;
  onSearchChange: (value: string) => void;
  archived: boolean;
  onArchivedChange: (value: boolean) => void;
  sessions: ReturnType<typeof useSessions>;
  groups: ReturnType<typeof groupSessions>;
}) {
  const { t } = useI18n();

  return (
    <>
      {/* Overlay chỉ tồn tại khi ngăn kéo đang mở, và chỉ trên mobile
          (`md:hidden` — desktop không bao giờ có lớp phủ). Là `<button>` thay
          `<div>` để bấm ra ngoài đóng ngăn kéo được bằng bàn phím. */}
      {open ? (
        <button
          type="button"
          aria-label={t("common.close")}
          className="fixed inset-0 z-20 bg-black/60 md:hidden"
          onClick={onClose}
        />
      ) : null}
      <aside
        data-testid="chat-sidebar"
        data-open={open || undefined}
        aria-label={t("chat.sessions")}
        className={`glass fixed inset-y-0 right-0 z-30 flex w-72 flex-col border-l border-border transition-transform duration-200 md:static md:translate-x-0 ${open ? "translate-x-0" : "translate-x-full"}`}
      >
        {/* `h-14`: cùng chiều cao TopNav nên kẻ ngang header thẳng hàng kẻ ngang nav. */}
        <div className="flex h-14 shrink-0 items-center justify-between gap-2 border-b border-border p-3">
          {/* Tiêu đề đổi theo chế độ đang xem: "Hội thoại" ↔ "Đã lưu trữ". */}
          <h2 className="text-base font-bold">
            {listProps.archived ? t("sessions.archived") : t("chat.sessions")}
          </h2>
          <button
            type="button"
            className="shrink-0 rounded-lg p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground md:hidden"
            onClick={onClose}
            aria-label={t("common.close")}
          >
            <XIcon className="size-5" aria-hidden="true" />
          </button>
        </div>
        <div className="flex shrink-0 flex-col p-3">
          <button
            type="button"
            onClick={onNewSession}
            disabled={creating}
            aria-label={t("chat.newChat")}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-brand px-3 py-2.5 text-sm font-semibold text-brand-ink hover:bg-brand-strong disabled:opacity-50"
          >
            <PlusIcon className="size-4 shrink-0" aria-hidden="true" />
            {creating ? t("common.loading") : t("chat.newChat")}
          </button>
          {actionError ? (
            <p className="mt-2 text-xs text-destructive" role="alert">
              {t("chat.newError")}
            </p>
          ) : null}
        </div>
        {/* `min-h-0`: `SessionList` có `<nav>` với `flex-1 overflow-y-auto`,
            cần một cha flex có chiều cao xác định thì nó mới cuộn được thay vì
            đẩy cả ngăn kéo tràn màn hình. */}
        <div className="flex min-h-0 flex-1 flex-col">
          <SessionList {...listProps} onDeleted={onDeleted} onNavigate={onNavigate} />
        </div>
      </aside>
    </>
  );
}
