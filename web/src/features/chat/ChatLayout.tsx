import { ArchiveIcon, ArchiveRestoreIcon, MoreHorizontalIcon, PencilIcon, Trash2Icon } from "lucide-react";
import { useMemo, useState } from "react";
import { NavLink, Outlet, useLocation, useNavigate } from "react-router";
import type { SessionDto } from "@/api/bindings";

import { ThemeToggle } from "@/components/ThemeToggle";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useLogout } from "@/features/auth/queries";
import { groupLabelKey, groupSessions } from "@/features/chat/sessionGroups";
import {
  useCreateSession,
  useDeleteSession,
  useSessions,
  useUpdateSession,
} from "@/features/sessions/queries";
import { useI18n } from "@/i18n";

export function ChatLayout() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const location = useLocation();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [archived, setArchived] = useState(false);
  const sessions = useSessions({ q: search, archived });
  // Trang chủ (HUB) không cần danh sách hội thoại: hội thoại là ngữ cảnh của
  // trang chat, đặt cạnh sơ đồ hệ thống chỉ làm loãng và tốn nửa màn hình.
  const showSessions = location.pathname !== "/";
  const createSession = useCreateSession();
  const logout = useLogout();
  const [actionError, setActionError] = useState(false);
  // Gom nhóm theo mốc thời gian. `useMemo` vì `Date.now()` trả về mỗi lần
  // render — nếu không, danh sách bị dựng lại liên tục và nhảy nhóm khi
  // người dùng đang đọc (tin chủ động hay về giữa lúc họ xem danh sách).
  const groups = useMemo(
    () => groupSessions(sessions.data?.sessions ?? [], new Date()),
    [sessions.data?.sessions],
  );

  async function newSession() {
    if (createSession.isPending) return;
    setActionError(false);
    try {
      const session = await createSession.mutateAsync({ title: null });
      setMobileOpen(false);
      navigate(`/sessions/${session.id}`);
    } catch {
      setActionError(true);
    }
  }

  async function signOut() {
    await logout.mutateAsync();
    navigate("/login", { replace: true });
  }

  function deleted(id: number) {
    setMobileOpen(false);
    if (location.pathname === `/sessions/${id}`) navigate("/");
  }

  return (
    <div className="flex min-h-svh bg-background text-foreground">
      <button
        type="button"
        className="fixed left-3 top-3 z-30 rounded-lg border border-border bg-card p-2 shadow-sm md:hidden"
        onClick={() => setMobileOpen((open) => !open)}
        aria-label={t("common.menu")}
        aria-expanded={mobileOpen}
      >
        ☰
      </button>
      {mobileOpen ? (
        <button
          type="button"
          aria-label={t("common.close")}
          className="fixed inset-0 z-20 bg-black/60 md:hidden"
          onClick={() => setMobileOpen(false)}
        />
      ) : null}
      <aside
        className={`fixed inset-y-0 left-0 z-20 flex w-72 -translate-x-full flex-col border-r border-border bg-card transition-transform md:static md:translate-x-0 ${mobileOpen ? "translate-x-0" : ""}`}
        aria-label={t("app.title")}
      >
        <div className="flex items-center justify-between border-b border-border p-4">
          <div>
            <p className="text-lg font-bold">Bean</p>
            <p className="text-xs text-muted-foreground">{t("app.tagline")}</p>
          </div>
          <button
            type="button"
            className="rounded-lg p-2 text-xl leading-none md:hidden"
            onClick={() => setMobileOpen(false)}
            aria-label={t("common.close")}
          >
            ×
          </button>
        </div>
        <nav className="space-y-1 border-b border-border p-3" aria-label={t("nav.main")}>
          {/* HUB là mục đầu tiên vì `/` (trang chủ) giờ hiện trạng thái cả hệ agent;
              "Trò chuyện" phải trỏ `/chat` vì `/` đã thuộc về HUB. */}
          <NavItem to="/" end label={t("nav.hub")} icon="◈" onNavigate={() => setMobileOpen(false)} />
          <NavItem to="/chat" label={t("nav.chat")} icon="◌" onNavigate={() => setMobileOpen(false)} />
          <NavItem to="/memory" label={t("nav.memory")} icon="◉" onNavigate={() => setMobileOpen(false)} />
          <NavItem to="/skills" label={t("nav.skills")} icon="◇" onNavigate={() => setMobileOpen(false)} />
          <NavItem to="/tasks" label={t("nav.tasks")} icon="◷" onNavigate={() => setMobileOpen(false)} />
          <NavItem to="/audit" label={t("nav.audit")} icon="≡" onNavigate={() => setMobileOpen(false)} />
          <NavItem to="/status" label={t("nav.status")} icon="●" onNavigate={() => setMobileOpen(false)} />
        </nav>
        {showSessions ? (
          <>
            <div className="border-b border-border p-3">
              <button
                type="button"
                onClick={() => void newSession()}
                disabled={createSession.isPending}
                className="flex w-full items-center justify-center gap-2 rounded-lg bg-brand px-3 py-2.5 text-sm font-semibold text-brand-ink hover:bg-brand-strong disabled:opacity-50"
              >
                <span aria-hidden="true">＋</span>
                {createSession.isPending ? t("common.loading") : t("chat.newChat")}
              </button>
              {actionError ? (
                <p className="mt-2 text-xs text-destructive" role="alert">
                  {t("chat.newError")}
                </p>
              ) : null}
            </div>
            <div className="flex items-center justify-between px-3 pt-3">
              <h2 className="text-xs font-bold uppercase tracking-wide text-muted-foreground">
                {archived ? t("sessions.archived") : t("chat.sessions")}
              </h2>
              <label className="flex items-center gap-1 text-xs text-muted-foreground">
                <input
                  type="checkbox"
                  checked={archived}
                  onChange={(event) => setArchived(event.target.checked)}
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
                onChange={(event) => setSearch(event.target.value)}
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
              {!sessions.isPending && !sessions.isError && sessions.data?.sessions.length === 0 ? (
                <p className="px-3 py-2 text-sm text-muted-foreground">
                  {search ? t("sessions.noResults") : t("chat.noSessions")}
                </p>
              ) : null}
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
                        onDeleted={() => deleted(session.id)}
                        onNavigate={() => setMobileOpen(false)}
                      />
                    ))}
                  </ul>
                </div>
              ))}
            </nav>
          </>
        ) : null}
        <div className="space-y-2 border-t border-border p-3">
          <LanguageSelect />
          <ThemeToggle className="w-full justify-start gap-2 px-2" />
          <button
            type="button"
            onClick={() => void signOut()}
            disabled={logout.isPending}
            className="w-full rounded-lg px-3 py-2 text-left text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            {logout.isPending ? t("common.loading") : t("chat.logout")}
          </button>
        </div>
      </aside>
      <main className="flex min-w-0 flex-1 flex-col">
        <Outlet />
      </main>
    </div>
  );
}

function NavItem({
  to,
  label,
  icon,
  end = false,
  onNavigate,
}: {
  to: string;
  label: string;
  icon: string;
  end?: boolean;
  onNavigate: () => void;
}) {
  return (
    <NavLink
      to={to}
      end={end}
      onClick={onNavigate}
      className={({ isActive }) =>
        `flex items-center gap-3 rounded-lg px-3 py-2 text-sm ${isActive ? "bg-accent font-semibold" : "text-muted-foreground hover:bg-background"}`
      }
    >
      <span aria-hidden="true" className="w-5 text-center">
        {icon}
      </span>
      {label}
    </NavLink>
  );
}

/**
 * Một dòng hội thoại trong sidebar.
 *
 * Export ra ngoài để test bám đúng component thật: test cần bấm menu Radix,
 * mà trong `ChatLayout` đầy đủ thì các request/portal khác của trang xen vào
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

function LanguageSelect() {
  const { t, lang, setLang } = useI18n();
  return (
    <label className="block text-xs font-medium text-muted-foreground">
      <span className="sr-only">{t("common.language")}</span>
      <select
        value={lang}
        onChange={(event) => setLang(event.target.value as "vi" | "en")}
        className="mt-1 w-full rounded-lg border border-border bg-card px-2 py-2 text-sm"
      >
        <option value="vi">Tiếng Việt</option>
        <option value="en">English</option>
      </select>
    </label>
  );
}
