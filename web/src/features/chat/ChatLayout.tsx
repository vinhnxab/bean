import { useState } from "react";
import { NavLink, Outlet, useNavigate } from "react-router";
import { useLogout } from "@/features/auth/queries";
import { useCreateSession, useSessions } from "@/features/sessions/queries";
import { useI18n } from "@/i18n";

export function ChatLayout() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const [mobileOpen, setMobileOpen] = useState(false);
  const sessions = useSessions();
  const createSession = useCreateSession();
  const logout = useLogout();

  async function newSession() {
    if (createSession.isPending) return;
    const session = await createSession.mutateAsync({ title: null });
    setMobileOpen(false);
    navigate(`/sessions/${session.id}`);
  }

  async function signOut() {
    await logout.mutateAsync();
    navigate("/login", { replace: true });
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
        aria-label={t("chat.sessions")}
      >
        <div className="flex items-center justify-between border-b border-slate-200 p-4 dark:border-slate-800">
          <div>
            <p className="text-lg font-bold">BeanAgent</p>
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
        <div className="p-3">
          <button
            type="button"
            onClick={newSession}
            disabled={createSession.isPending}
            className="flex w-full items-center justify-center gap-2 rounded-xl bg-emerald-600 px-3 py-2.5 text-sm font-semibold text-white hover:bg-emerald-700 disabled:opacity-50"
          >
            <span aria-hidden="true">＋</span>
            {createSession.isPending ? t("common.loading") : t("chat.newChat")}
          </button>
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
            <p className="px-3 py-2 text-sm text-slate-500">{t("chat.noSessions")}</p>
          ) : null}
          <ul className="space-y-1">
            {sessions.data?.sessions.map((session) => (
              <li key={session.id}>
                <NavLink
                  to={`/sessions/${session.id}`}
                  onClick={() => setMobileOpen(false)}
                  className={({ isActive }) =>
                    `block truncate rounded-lg px-3 py-2 text-sm ${isActive ? "bg-emerald-100 font-semibold text-emerald-900 dark:bg-emerald-950 dark:text-emerald-200" : "hover:bg-slate-100 dark:hover:bg-slate-800"}`
                  }
                >
                  {session.title || t("chat.untitled")}
                </NavLink>
              </li>
            ))}
          </ul>
        </nav>
        <div className="space-y-2 border-t border-slate-200 p-3 dark:border-slate-800">
          <LanguageSelect />
          <button
            type="button"
            onClick={signOut}
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
