import { ArchiveIcon, ArchiveRestoreIcon, MoreHorizontalIcon, PencilIcon, Trash2Icon } from "lucide-react";
import { useState } from "react";
import { NavLink } from "react-router";
import type { SessionDto } from "@/api/bindings";

import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { groupLabelKey, type groupSessions } from "@/features/chat/sessionGroups";
import { useDeleteSession, type useSessions, useUpdateSession } from "@/features/sessions/queries";
import { useI18n } from "@/i18n";
import { Stagger } from "@/lib/anim";

/**
 * Danh sách hội thoại — tiêu đề ("Hội thoại" / "Đã lưu trữ") do khung chứa
 * (`RightSidebar`) in, component này chỉ lo hàng công cụ và các dòng.
 *
 * Tách riêng khỏi `ChatLayout` để sidebar phải và test **dùng chung đúng một
 * bản**: hai bản copy chắc chắn lệch nhau sau vài lần sửa.
 */
export function SessionList({
  search,
  onSearchChange,
  archived,
  onArchivedChange,
  sessions,
  groups,
  onDeleted,
  onNavigate,
}: {
  search: string;
  onSearchChange: (value: string) => void;
  archived: boolean;
  onArchivedChange: (value: boolean) => void;
  sessions: ReturnType<typeof useSessions>;
  groups: ReturnType<typeof groupSessions>;
  onDeleted: (id: number) => void;
  onNavigate: () => void;
}) {
  const { t } = useI18n();
  const empty = !sessions.isPending && !sessions.isError && sessions.data?.sessions.length === 0;

  return (
    <>
      <div className="flex items-center justify-end px-3 pt-3">
        <label className="flex items-center gap-1 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={archived}
            onChange={(event) => onArchivedChange(event.target.checked)}
          />
          {t("sessions.showArchived")}
        </label>
      </div>
      <div className="px-3 py-2">
        <label className="sr-only" htmlFor="session-search">
          {t("sessions.search")}
        </label>
        <input
          id="session-search"
          type="search"
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          placeholder={t("sessions.searchPlaceholder")}
          className="w-full rounded-lg border border-border bg-card px-2 py-2 text-sm outline-none focus:border-live"
        />
      </div>
      <nav className="flex-1 overflow-y-auto px-2 pb-2" aria-label={t("chat.sessions")}>
        {sessions.isPending ? (
          <p className="px-3 py-2 text-sm text-muted-foreground">{t("chat.loading")}</p>
        ) : null}
        {sessions.isError ? (
          <p className="px-3 py-2 text-sm text-alert" role="alert">
            {t("chat.historyError")}
          </p>
        ) : null}
        {empty ? (
          <p className="px-3 py-2 text-sm text-muted-foreground">
            {search ? t("sessions.noResults") : t("chat.noSessions")}
          </p>
        ) : null}
        {/* `Stagger` bọc ngoài `ul` (không bọc trong): `ul` phải giữ đúng con là
            `li` — wrapper nội bộ sẽ làm hỏng cấu trúc danh sách. Selector `li`
            chọn từng dòng dù chúng nằm trong nhiều nhóm. */}
        <Stagger item="li" className="pb-2">
          {groups.map((group) => (
            <div key={group.key} className="mb-1">
              {/* Nhãn nhóm: nhỏ, đậm, chữ thường (không IN HOA — `uppercase`
                  làm tiếng Việt có dấu nhảy dấu và khó đọc hơn). */}
              <h3 className="px-3 pb-1 pt-3 text-xs font-semibold text-muted-foreground">
                {t(groupLabelKey(group.key))}
              </h3>
              <ul className="space-y-0.5">
                {group.sessions.map((session) => (
                  <SessionRow
                    key={session.id}
                    session={session}
                    onDeleted={() => onDeleted(session.id)}
                    onNavigate={onNavigate}
                  />
                ))}
              </ul>
            </div>
          ))}
        </Stagger>
      </nav>
    </>
  );
}

/**
 * Một dòng hội thoại.
 *
 * Export ra ngoài để test bám đúng component thật: test cần bấm menu Radix,
 * mà trong khung đầy đủ thì các request/portal khác của trang xen vào
 * làm test chập chờn (test chạy đơn lẻ xanh, chạy cả file thì fail ở menu
 * thứ hai). Render riêng `SessionRow` loại bỏ hẳn nhiễu đó.
 */
export function SessionRow({
  session,
  onDeleted,
  onNavigate,
}: {
  session: SessionDto;
  onDeleted: () => void;
  onNavigate: () => void;
}) {
  const { t } = useI18n();
  const update = useUpdateSession();
  const remove = useDeleteSession();
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(session.title);
  const [deleteOpen, setDeleteOpen] = useState(false);

  async function saveTitle() {
    const value = title.trim();
    if (!value || value === session.title) {
      setEditing(false);
      return;
    }
    try {
      await update.mutateAsync({ id: session.id, request: { title: value, archived: null } });
      setEditing(false);
    } catch {
      // Keep the editor open so the title is not silently lost.
    }
  }

  async function confirmDelete() {
    try {
      await remove.mutateAsync(session.id);
      onDeleted();
    } catch {
      // Keep the confirmation open on failure.
    }
  }

  async function toggleArchive() {
    try {
      await update.mutateAsync({
        id: session.id,
        request: { title: null, archived: !session.archived },
      });
    } catch {
      // The next refetch remains the source of truth.
    }
  }

  return (
    <li className="group/session relative">
      {editing ? (
        <div className="flex gap-1 p-1">
          <label className="sr-only" htmlFor={`rename-${session.id}`}>
            {t("common.rename")}
          </label>
          <input
            id={`rename-${session.id}`}
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void saveTitle();
            }}
            aria-label={t("sessions.rename")}
            className="min-w-0 flex-1 rounded-md border border-border bg-surface-raised px-2 py-1.5 text-sm outline-none focus-visible:border-brand"
          />
          <button
            type="button"
            onClick={() => void saveTitle()}
            className="rounded-md bg-brand px-2 py-1 text-xs font-medium text-brand-ink hover:bg-brand-strong"
          >
            {t("common.save")}
          </button>
        </div>
      ) : (
        <div className="flex items-center">
          <NavLink
            to={`/sessions/${session.id}`}
            onClick={onNavigate}
            className={({ isActive }) =>
              `min-w-0 flex-1 truncate rounded-md py-2 pl-3 pr-2 text-sm transition-colors ${isActive ? "brand-wash font-semibold text-brand" : "text-ink-muted hover:bg-surface-hover hover:text-ink"}`
            }
          >
            {session.title || t("chat.untitled")}
          </NavLink>
          {/* Một nút menu thay cho ba nút luôn hiện. Lý do: ở bản cũ mỗi dòng
              chiếm ~60px cho ✎ ▱ ×, gần bằng cả tiêu đề, và ba nút sáng suốt
              khiến danh sách trông như bảng điều khiển thay vì danh sách đọc.
              Menu ẩn khi không rê, hiện khi rê hoặc khi focus (bàn phím). */}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                aria-label={t("sessions.moreActions")}
                className="mr-1 flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-ink focus-visible:opacity-100 group-hover/session:opacity-100"
              >
                <MoreHorizontalIcon className="size-4" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem
                onSelect={() => {
                  setTitle(session.title);
                  setEditing(true);
                }}
              >
                <PencilIcon className="size-3.5" />
                {t("sessions.rename")}
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => void toggleArchive()}>
                {session.archived ? (
                  <ArchiveRestoreIcon className="size-3.5" />
                ) : (
                  <ArchiveIcon className="size-3.5" />
                )}
                {session.archived ? t("sessions.unarchive") : t("sessions.archive")}
              </DropdownMenuItem>
              <DropdownMenuItem className="text-alert focus:text-alert" onSelect={() => setDeleteOpen(true)}>
                <Trash2Icon className="size-3.5" />
                {t("common.delete")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      )}
      <ConfirmDialog
        open={deleteOpen}
        title={t("sessions.deleteTitle")}
        message={`${t("sessions.deleteMessage")}\n${session.title || t("chat.untitled")}`}
        confirmLabel={t("common.delete")}
        cancelLabel={t("common.cancel")}
        danger
        pending={remove.isPending}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleteOpen(false)}
      />
    </li>
  );
}
