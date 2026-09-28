import { useState } from "react";
import { NavLink, Outlet, useLocation, useNavigate } from "react-router";
import type { SessionDto } from "@/api/bindings";

import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { useLogout } from "@/features/auth/queries";
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
    <div className="flex min-h-svh bg-slate-50 text-slate-900 dark:bg-slate-950 dark:text-slate-100">
      <button
        type="button"
        className="fixed left-3 top-3 z-30 rounded-lg border border-slate-300 bg-white p-2 shadow-sm md:hidden dark:border-slate-700 dark:bg-slate-900"
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
          className="fixed inset-0 z-20 bg-slate-950/40 md:hidden"
          onClick={() => setMobileOpen(false)}
        />
      ) : null}
      <aside
        className={`fixed inset-y-0 left-0 z-20 flex w-72 -translate-x-full flex-col border-r border-slate-200 bg-white transition-transform md:static md:translate-x-0 dark:border-slate-800 dark:bg-slate-900 ${mobileOpen ? "translate-x-0" : ""}`}
        aria-label={t("app.title")}
      >
        <div className="flex items-center justify-between border-b border-slate-200 p-4 dark:border-slate-800">
          <div>
            <p className="text-lg font-bold">Bean</p>
            <p className="text-xs text-slate-500 dark:text-slate-400">{t("app.tagline")}</p>
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
        <nav
          className="space-y-1 border-b border-slate-200 p-3 dark:border-slate-800"
          aria-label={t("nav.main")}
        >
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
            <div className="border-b border-slate-200 p-3 dark:border-slate-800">
              <button
                type="button"
                onClick={() => void newSession()}
                disabled={createSession.isPending}
                className="flex w-full items-center justify-center gap-2 rounded-xl bg-emerald-600 px-3 py-2.5 text-sm font-semibold text-white hover:bg-emerald-700 disabled:opacity-50"
              >
                <span aria-hidden="true">＋</span>
                {createSession.isPending ? t("common.loading") : t("chat.newChat")}
              </button>
              {actionError ? (
                <p className="mt-2 text-xs text-rose-600" role="alert">
                  {t("chat.newError")}
                </p>
              ) : null}
            </div>
            <div className="flex items-center justify-between px-3 pt-3">
              <h2 className="text-xs font-bold uppercase tracking-wide text-slate-500 dark:text-slate-400">
                {archived ? t("sessions.archived") : t("chat.sessions")}
              </h2>
              <label className="flex items-center gap-1 text-xs text-slate-500 dark:text-slate-400">
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
                className="w-full rounded-lg border border-slate-300 bg-white px-2 py-2 text-sm outline-none focus:border-emerald-500 dark:border-slate-700 dark:bg-slate-950"
              />
            </div>
            <nav className="flex-1 overflow-y-auto px-2" aria-label={t("chat.sessions")}>
              {sessions.isPending ? (
                <p className="px-3 py-2 text-sm text-slate-500">{t("chat.loading")}</p>
              ) : null}
              {sessions.isError ? (
                <p className="px-3 py-2 text-sm text-rose-600" role="alert">
                  {t("chat.historyError")}
                </p>
              ) : null}
              {!sessions.isPending && !sessions.isError && sessions.data?.sessions.length === 0 ? (
                <p className="px-3 py-2 text-sm text-slate-500">
                  {search ? t("sessions.noResults") : t("chat.noSessions")}
                </p>
              ) : null}
              <ul className="space-y-1">
                {sessions.data?.sessions.map((session) => (
                  <SessionRow
                    key={session.id}
                    session={session}
                    onDeleted={() => deleted(session.id)}
                    onNavigate={() => setMobileOpen(false)}
                  />
                ))}
              </ul>
            </nav>
          </>
        ) : null}
        <div className="space-y-2 border-t border-slate-200 p-3 dark:border-slate-800">
          <LanguageSelect />
          <button
            type="button"
            onClick={() => void signOut()}
            disabled={logout.isPending}
            className="w-full rounded-lg px-3 py-2 text-left text-sm text-slate-600 hover:bg-slate-100 disabled:opacity-50 dark:text-slate-300 dark:hover:bg-slate-800"
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
        `flex items-center gap-3 rounded-lg px-3 py-2 text-sm ${isActive ? "bg-slate-100 font-semibold dark:bg-slate-800" : "text-slate-600 hover:bg-slate-50 dark:text-slate-300 dark:hover:bg-slate-800"}`
      }
    >
      <span aria-hidden="true" className="w-5 text-center">
        {icon}
      </span>
      {label}
    </NavLink>
  );
}

function SessionRow({
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
    <li className="rounded-lg border border-transparent hover:border-slate-200 dark:hover:border-slate-700">
      {editing ? (
        <div className="flex gap-1 p-1">
          <label className="sr-only" htmlFor={`rename-${session.id}`}>
            {t("sessions.rename")}
          </label>
          <input
            id={`rename-${session.id}`}
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void saveTitle();
            }}
            className="min-w-0 flex-1 rounded-md border border-slate-300 px-2 py-1 text-sm dark:border-slate-700 dark:bg-slate-950"
          />
          <button
            type="button"
            onClick={() => void saveTitle()}
            className="rounded-md bg-emerald-600 px-2 text-xs text-white"
          >
            {t("common.save")}
          </button>
        </div>
      ) : (
        <div className="flex items-center gap-1 p-1">
          <NavLink
            to={`/sessions/${session.id}`}
            onClick={onNavigate}
            className={({ isActive }) =>
              `min-w-0 flex-1 truncate rounded-md px-2 py-1.5 text-sm ${isActive ? "bg-emerald-100 font-semibold text-emerald-900 dark:bg-emerald-950 dark:text-emerald-200" : ""}`
            }
          >
            {session.title || t("chat.untitled")}
          </NavLink>
          <button
            type="button"
            onClick={() => {
              setTitle(session.title);
              setEditing(true);
            }}
            aria-label={`${t("sessions.rename")} ${session.title || t("chat.untitled")}`}
            className="rounded-md px-1.5 py-1 text-xs text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-800"
          >
            ✎
          </button>
          <button
            type="button"
            onClick={() => void toggleArchive()}
            aria-label={session.archived ? t("sessions.unarchive") : t("sessions.archive")}
            className="rounded-md px-1.5 py-1 text-xs text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-800"
          >
            {session.archived ? "↥" : "▱"}
          </button>
          <button
            type="button"
            onClick={() => setDeleteOpen(true)}
            aria-label={`${t("common.delete")} ${session.title || t("chat.untitled")}`}
            className="rounded-md px-1.5 py-1 text-xs text-rose-600 hover:bg-rose-50 dark:hover:bg-rose-950"
          >
            ×
          </button>
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
    <label className="block text-xs font-medium text-slate-500 dark:text-slate-400">
      <span className="sr-only">{t("common.language")}</span>
      <select
        value={lang}
        onChange={(event) => setLang(event.target.value as "vi" | "en")}
        className="mt-1 w-full rounded-lg border border-slate-300 bg-white px-2 py-2 text-sm dark:border-slate-700 dark:bg-slate-950"
      >
        <option value="vi">Tiếng Việt</option>
        <option value="en">English</option>
      </select>
    </label>
  );
}
