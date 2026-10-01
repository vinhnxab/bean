import {
  ActivityIcon,
  ArchiveIcon,
  ArchiveRestoreIcon,
  BrainIcon,
  CalendarClockIcon,
  GaugeIcon,
  HistoryIcon,
  LanguagesIcon,
  LogOutIcon,
  type LucideIcon,
  MenuIcon,
  MessageSquareIcon,
  MoreHorizontalIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  PencilIcon,
  PlugIcon,
  PlusIcon,
  ScrollTextIcon,
  SparklesIcon,
  Trash2Icon,
  WrenchIcon,
  XIcon,
} from "lucide-react";
import { Suspense, useEffect, useMemo, useState } from "react";
import { NavLink, Outlet, useLocation, useMatch, useNavigate } from "react-router";
import type { SessionDto } from "@/api/bindings";

import { ThemeToggle } from "@/components/ThemeToggle";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { Dialog, DialogClose, DialogContent } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { RouteFallback } from "@/components/ui/Page";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useLogout } from "@/features/auth/queries";
import { groupLabelKey, groupSessions } from "@/features/chat/sessionGroups";
import {
  useCreateSession,
  useDeleteSession,
  useSessions,
  useUpdateSession,
} from "@/features/sessions/queries";
import { type Lang, useI18n } from "@/i18n";
import type { MessageKey } from "@/i18n/vi";
import { useSidebarCollapsed } from "@/lib/sidebar";

/**
 * Mục điều hướng chính.
 *
 * Icon lấy từ `lucide-react` (đã có sẵn trong `package.json`, không tải thêm tài
 * nguyên ngoài — agents.md mục 0.9). Trước đây là ký tự Unicode `◈ ◌ ◉ ◇ ◷ ⚙
 * ⧉ ≡ ●`: chúng **đổi hình dáng theo font của hệ điều hành**, nên cùng một mục
 * trông khác nhau giữa Windows/macOS/Linux, và nét mảnh biến mất ở kích thước nhỏ.
 * Icon vector vẽ bằng `currentColor` nên kế thừa đúng màu chữ của vùng chứa.
 *
 * Khai báo **một bảng** thay vì rải mười lệnh `NavItem` rời rạc: nhãn lấy từ
 * từ điển qua `labelKey`, nên thêm/bớt mục là thêm/bớt một dòng ở đây, không phải
 * sửa cả khối JSX cho cân bằng thẩm mỹ.
 */
const NAV_ITEMS: { to: string; labelKey: MessageKey; icon: LucideIcon; end?: boolean }[] = [
  // HUB là mục đầu tiên vì `/` (trang chủ) hiện trạng thái cả hệ agent; "Trò chuyện"
  // phải trỏ `/chat` vì `/` đã thuộc về HUB.
  { to: "/", labelKey: "nav.hub", icon: GaugeIcon, end: true },
  { to: "/chat", labelKey: "nav.chat", icon: MessageSquareIcon },
  { to: "/memory", labelKey: "nav.memory", icon: BrainIcon },
  { to: "/skills", labelKey: "nav.skills", icon: SparklesIcon },
  { to: "/tasks", labelKey: "nav.tasks", icon: CalendarClockIcon },
  { to: "/tools", labelKey: "nav.tools", icon: WrenchIcon },
  { to: "/mcp", labelKey: "nav.mcp", icon: PlugIcon },
  { to: "/audit", labelKey: "nav.audit", icon: ScrollTextIcon },
  { to: "/status", labelKey: "nav.status", icon: ActivityIcon },
];

export function ChatLayout() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const location = useLocation();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [collapsed, toggleCollapsed] = useSidebarCollapsed();
  // Chỉ mở được khi rail đang thu gọn: mở rộng thì danh sách đã nằm sẵn trong
  // sidebar, hai thứ cùng hiện là trùng lặp. `useEffect` đóng flyout khi bung
  // sidebar ra — nếu không, bảng phủ lên chính danh sách vừa lộ ra.
  const [historyOpen, setHistoryOpen] = useState(false);
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

  // Bung sidebar ra thì đóng flyout: hai bảng danh sách cùng hiện sẽ chồng lên
  // nhau ở cùng một mép trái. Chuyển hội thoại rồi quay lại HUB cũng đóng, vì
  // HUB vốn không có danh sách để flyout đại diện.
  useEffect(() => {
    if (!collapsed || !showSessions) setHistoryOpen(false);
  }, [collapsed, showSessions]);

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
    // `h-svh overflow-hidden`: khung ứng dụng cao đúng một màn hình và **không để
    // trình duyệt cuộn**. Nội dung từng màn cuộn bên trong `<main>` (bên dưới),
    // còn sidebar tự cuộn trong `<nav>`. Dùng `min-h-svh` như trước thì khung cao
    // theo nội dung, trang dài sẽ đẩy cả thanh địa chỉ lẫn sidebar — sai ý đồ.
    <div className="flex h-svh overflow-hidden bg-background text-foreground">
      <button
        type="button"
        className="fixed left-3 top-3 z-30 rounded-lg border border-border bg-card p-2 shadow-sm md:hidden"
        onClick={() => setMobileOpen((open) => !open)}
        aria-label={t("common.menu")}
        aria-expanded={mobileOpen}
      >
        <MenuIcon className="size-5" aria-hidden="true" />
      </button>
      {mobileOpen ? (
        <button
          type="button"
          aria-label={t("common.close")}
          className="fixed inset-0 z-20 bg-black/60 md:hidden"
          onClick={() => setMobileOpen(false)}
        />
      ) : null}
      {/*
        Rail thu gọn = `md:w-20`. Ở `< md` vẫn `w-72` và `md:` bị bỏ qua nên
        ngăn kéo trên điện thoại không bao giờ thành rail icon (xem `lib/sidebar.ts`).
      */}
      <aside
        className={`fixed inset-y-0 left-0 z-20 flex w-72 -translate-x-full flex-col border-r border-border bg-card transition-[width,transform] duration-200 md:static md:translate-x-0 ${collapsed ? "md:w-20" : "md:w-72"} ${mobileOpen ? "translate-x-0" : ""}`}
        aria-label={t("app.title")}
        data-testid="sidebar-shell"
      >
        {/* `hidden md:flex`: khi thu gọn, đầu thanh chỉ còn nút mở lại — người
            dùng không phải cuộn lên đầu sidebar mới tìm thấy nút.

            Nút ở trạng thái thu gọn được **nền đậm hơn** (`bg-accent`): đó là
            thứ duy nhất đưa sidebar trở lại, mà trên nền tối một icon
            `text-muted-foreground` dễ bị bỏ qua trong lúc mắt đang ở nội dung.

            `justify-center` khi thu gọn: `justify-between` sẽ đẩy nút sang mép
            trái khi không còn khối tiêu đề bên cạnh — mất cảm giác "cột icon". */}
        <div
          className={`hidden items-center gap-2 border-b border-border p-3 md:flex ${collapsed ? "justify-center" : "justify-between"}`}
        >
          {!collapsed ? (
            <div className="min-w-0">
              <p className="text-lg font-bold">Bean</p>
              <p className="truncate text-xs text-muted-foreground">{t("app.tagline")}</p>
            </div>
          ) : null}
          <button
            type="button"
            onClick={toggleCollapsed}
            // Nhãn mô tả **hành động sắp làm**, không phải trạng thái hiện tại,
            // đúng như quy ước của `ThemeToggle`.
            aria-label={collapsed ? t("nav.expand") : t("nav.collapse")}
            aria-expanded={!collapsed}
            title={collapsed ? t("nav.expand") : t("nav.collapse")}
            data-slot="sidebar-toggle"
            className={`shrink-0 rounded-lg p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 ${collapsed ? "bg-accent text-foreground" : ""}`}
          >
            {collapsed ? (
              <PanelLeftOpenIcon className="size-4" aria-hidden="true" />
            ) : (
              <PanelLeftCloseIcon className="size-4" aria-hidden="true" />
            )}
          </button>
        </div>
        {/* Nút đóng ngăn kéo ở đầu sidebar: chỉ có ở `< md`. */}
        <div className="flex items-center justify-between gap-2 border-b border-border p-3 md:hidden">
          <div className="min-w-0">
            <p className="text-lg font-bold">Bean</p>
            <p className="truncate text-xs text-muted-foreground">{t("app.tagline")}</p>
          </div>
          <button
            type="button"
            className="shrink-0 rounded-lg p-2 text-muted-foreground hover:bg-accent hover:text-foreground"
            onClick={() => setMobileOpen(false)}
            aria-label={t("common.close")}
          >
            <XIcon className="size-5" aria-hidden="true" />
          </button>
        </div>
        {/* `TooltipProvider` bọc riêng `<nav>`: Radix `Tooltip.Root` bắt buộc
            phải có provider tổ tiên. Chỉ rail thu gọn mới dùng tooltip, nhưng
            provider đặt ở đây rẻ hơn và không phụ thuộc vào việc mục nào
            đang thu gọn. */}
        <TooltipProvider>
          <nav
            // `flex flex-col items-center` (khi thu gọn) là thứ **thực sự** canh
            // giữa các mục: `<a>` mặc định là `display:inline`, mà `inline` thì
            // `mx-auto` không có tác dụng — đo `getComputedStyle` ra
            // `display:inline`, `margin: 0px/0px`. `w-full` khi mở rộng để mục
            // trải hết chiều ngang như bản cũ.
            //
            // `gap-1` thay cho `space-y-1` cũ: `space-y` đặt `margin-top` lên mọi
            // con trừ con đầu, nên nó phụ thuộc con có phải block hay không — đúng
            // thứ vừa hỏng. `gap` là thuộc tính của cha, không dựa vào display của
            // con, nên ổn định ở cả hai trạng thái.
            // `md:flex-1` để phần điều hướng **hấp thụ chỗ trống** giữa nó và footer
            // (`mt-auto`). Nhờ vậy footer luôn ở đáy ở cả hai trạng thái: bản mở
            // rộng có `SessionList` cuộn riêng, bản thu gọn thì chỉ có `nav` — mà
            // `flex-1` thì nốt.
            //
            // `gap-1` thay cho `space-y-1` cũ: `space-y` đặt `margin-top` lên mọi
            // con trừ con đầu, nên nó phụ thuộc con có phải block hay không — đúng
            // thứ vừa hỏng. `gap` là thuộc tính của cha, không dựa vào display của
            // con, nên ổn định ở cả hai trạng thái.
            //
            // `md:flex-1` để phần điều hướng **hấp thụ chỗ trống** giữa nó và footer
            // (`mt-auto`). Nhờ vậy footer luôn ở đáy ở cả hai trạng thái: bản mở
            // rộng có `SessionList` cuộn riêng, bản thu gọn thì chỉ có `nav` — mà
            // `flex-1` thì nốt.
            className={`flex flex-col gap-1 border-b border-border md:flex-1 ${collapsed ? "items-center p-2" : "p-3"}`}
            aria-label={t("nav.main")}
          >
            {NAV_ITEMS.map((item) => (
              <NavItem
                key={item.to}
                to={item.to}
                end={item.end}
                label={t(item.labelKey)}
                icon={item.icon}
                collapsed={collapsed}
                onNavigate={() => setMobileOpen(false)}
              />
            ))}
          </nav>
        </TooltipProvider>
        {showSessions ? (
          <>
            {/*
              Rail thu gọn giữ lại **hai hành động** làm được bằng một icon:
              tạo hội thoại mới, và mở danh sách (flyout). Phần danh sách **ẩn
              hẳn** — `md:hidden`, KHÔNG phải `md:contents`: `display:contents`
              khiến phần con vẫn hiện và tràn ra ngoài rail 80px.
            */}
            <div
              className={`flex border-b border-border ${collapsed ? "flex-col items-center gap-2 px-2 py-2" : "flex-col p-3"}`}
            >
              <button
                type="button"
                onClick={() => void newSession()}
                disabled={createSession.isPending}
                aria-label={t("chat.newChat")}
                title={collapsed ? t("chat.newChat") : undefined}
                className={`flex items-center justify-center gap-2 rounded-lg bg-brand font-semibold text-brand-ink hover:bg-brand-strong disabled:opacity-50 ${collapsed ? "size-9" : "w-full px-3 py-2.5 text-sm"}`}
              >
                <PlusIcon className="size-4 shrink-0" aria-hidden="true" />
                {collapsed ? (
                  <span className="sr-only">{t("chat.newChat")}</span>
                ) : createSession.isPending ? (
                  t("common.loading")
                ) : (
                  t("chat.newChat")
                )}
              </button>
              {/*
                Icon lịch sử **chỉ có ở rail thu gọn**. Ở sidebar mở rộng, danh
                sách đã nằm sẵn ngay dưới nên một nút mở lại nó là thừa.
              */}
              {collapsed ? (
                <button
                  type="button"
                  onClick={() => setHistoryOpen((open) => !open)}
                  aria-expanded={historyOpen}
                  aria-label={t("sessions.openInSidebar")}
                  title={t("sessions.openInSidebar")}
                  data-slot="history-toggle"
                  className={`flex size-9 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground ${historyOpen ? "bg-accent text-foreground" : ""}`}
                >
                  <HistoryIcon className="size-4 shrink-0" aria-hidden="true" />
                  <span className="sr-only">{t("sessions.openInSidebar")}</span>
                </button>
              ) : null}
              {actionError ? (
                <p className="text-xs text-destructive" role="alert">
                  {t("chat.newError")}
                </p>
              ) : null}
            </div>
            {/*
              Danh sách chỉ sống ở **một** nơi tại một thời điểm: sidebar khi mở
              rộng, flyout khi thu gọn.

              Không dùng `md:hidden` để "ẩn bằng CSS" rồi vẫn render cả hai:
              đó là hai `<nav>` trùng nhãn và **hai ô input cùng `id="session-search"`**,
              khiến `<label for>` trỏ nhầm phần tử và trình đọc màn hình đọc hai
              bản. Giữ `SessionList` ở một nhánh `? :` là rẻ hơn và đúng hơn.

              Ở `< md` ngăn kéo vẫn dùng nút lịch sử + flyout: flyout rộng 320px,
              vừa khít màn hình điện thoại, và `mousedown` ngoài đã đóng nó sẵn.
            */}
            {!collapsed ? (
              <SessionList
                search={search}
                onSearchChange={setSearch}
                archived={archived}
                onArchivedChange={setArchived}
                sessions={sessions}
                groups={groups}
                onDeleted={deleted}
                onNavigate={() => setMobileOpen(false)}
              />
            ) : null}
          </>
        ) : null}
        {/*
            Chân sidebar: ngôn ngữ và chủ đề, đăng xuất.

            # Vì sao `mt-auto`

            `<aside>` là `flex flex-col`. Khi thu gọn, `SessionList` **không render**
            (danh sách nằm trong flyout), nên phần giữa chỉ còn `nav` — footer lập tức
            trôi lên sát `nav` và treo lơ lửng giữa màn hình. `mt-auto` đẩy nó xuống
            đáy, đúng vị trí của nó ở bản mở rộng: chân luôn ở đáy, chuyển trạng
            thái không phải chuyển vị trí. `nav` phía trên giữ `flex-1` + cuộn riêng
            nên danh sách dài vẫn không đẩy footer xuống dưới màn hình.

            # Vì sao bỏ padding khi thu gọn

            Rail 80px trừ `p-3` (12px mỗi bên) còn **56px**. `Select` và nút chủ đề
            đều là ô vuông 32px nên vẫn vừa, nhưng nút **đăng xuất** ở trạng thái
            mở rộng có `px-2 py-2` — giữ padding thì chân sidebar lệch vài pixel so
            với các mục điều hướng ngay trên (vốn dùng `p-2`). Bỏ padding và canh
            giữa bằng `mx-auto` cho cả ba nút, thành một cột icon thẳng hàng.

            # Vì sao XẾP DỌC ở cả hai trạng thái

            Sau khi thêm **nhãn chữ**, một hàng ngang không còn đủ chỗ: "Ngôn ngữ ·
            Tiếng Việt" cạnh "Giao diện" bị `truncate` thành "Ngôn…"/"Giá…", tệ hơn
            không có nhãn. Xếp dọc giữ được cả hai nhãn đầy đủ.
        */}
        <div
          className={`mt-auto flex flex-col border-t border-border ${collapsed ? "items-center gap-1 p-2" : "space-y-1 p-3"}`}
        >
          <LanguageSelect collapsed={collapsed} />
          <ThemeToggle
            className={
              collapsed
                ? "size-8 shrink-0"
                : "w-full justify-start gap-2 px-2 text-sm text-muted-foreground hover:text-foreground"
            }
            // Nhãn trợ năng **chứa** chữ đang hiện (`WCAG 2.5.3 Label in Name`) rồi
            // mới kèm trạng thái và hành động: "Giao diện: Tối, Chuyển sang giao
            // diện sáng". Bỏ chữ đầu thì trình đọc màn hình đọc một câu không liên
            // quan tới thứ đang hiện trên màn hình.
            label={collapsed ? undefined : t("theme.label")}
          />
          <button
            type="button"
            onClick={() => void signOut()}
            disabled={logout.isPending}
            aria-label={collapsed ? t("chat.logout") : undefined}
            title={collapsed ? t("chat.logout") : undefined}
            className={`flex items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50 ${collapsed ? "size-8 shrink-0 justify-center" : "w-full gap-3 px-2 py-2 text-left text-sm"}`}
          >
            <LogOutIcon className="size-4 shrink-0" aria-hidden="true" />
            {collapsed ? (
              <span className="sr-only">{t("chat.logout")}</span>
            ) : logout.isPending ? (
              t("common.loading")
            ) : (
              t("chat.logout")
            )}
          </button>
        </div>
      </aside>
      <main className="flex min-w-0 flex-1 flex-col overflow-y-auto">
        {/* `Suspense` đặt quanh `Outlet` chứ không quanh cả `ChatLayout` (ở `App.tsx`):
            khi đang tải chunk của màn, chỉ vùng nội dung được thay bằng skeleton —
            sidebar vẫn đứng yên. Bọc ở ngoài sẽ làm cả khung giao diện biến mất rồi
            nhảy lại, nhấp nháy rõ rệt khi bấm chuyển màn trên mạng chậm. */}
        <Suspense fallback={<RouteFallback />}>
          <Outlet />
        </Suspense>
      </main>
      {showSessions ? (
        <HistoryFlyout
          open={collapsed && historyOpen}
          onClose={() => setHistoryOpen(false)}
          search={search}
          onSearchChange={setSearch}
          archived={archived}
          onArchivedChange={setArchived}
          sessions={sessions}
          groups={groups}
          onDeleted={deleted}
          onNavigate={() => setMobileOpen(false)}
        />
      ) : null}
    </div>
  );
}

/**
 * Danh sách hội thoại — tách riêng để sidebar và flyout **dùng chung đúng một
 * bản**. Trước đây nội dung này nằm thẳng trong JSX của `ChatLayout`; muốn cho
 * rail thu gọn mở được danh sách thì phải nhân bản, và hai bản chắc chắn lệch
 * nhau sau vài lần sửa.
 */
function SessionList({
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
      <div className="flex items-center justify-between px-3 pt-3">
        <h2 className="text-xs font-bold uppercase tracking-wide text-muted-foreground">
          {archived ? t("sessions.archived") : t("chat.sessions")}
        </h2>
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
      </nav>
    </>
  );
}

/**
 * Flyout danh sách hội thoại cho rail thu gọn.
 *
 * # Vì sao KHÔNG dùng modal
 *
 * Modal che mất cuộc hội thoại đang mở, và "đổi hội thoại" là **điều hướng**,
 * không phải quyết định cần chặn lại. Flyout neo ngay cạnh rail nên người dùng
 * vẫn đọc được nội dung chat ở bên phải trong lúc chọn — cùng cách VS Code mở
 * danh sách hội thoại khi sidebar bị thu gọn.
 *
 * # Vì sao dùng Radix `Dialog` với `modal={false}`
 *
 * Đây là **popover không modal**: Radix vẫn lo bẫy focus, trả focus về nút đã
 * bấm, đóng bằng Esc và chặn Tab ra ngoài — mà không cần thêm gói
 * `@radix-ui/react-popover` (agents.md mục 15.10: không thêm dependency thừa).
 */
function HistoryFlyout({
  open,
  onClose,
  ...listProps
}: {
  open: boolean;
  onClose: () => void;
} & Parameters<typeof SessionList>[0]) {
  const { t } = useI18n();

  // Ở chế độ `modal={false}`, Radix **không** tự đóng khi bấm ra ngoài. Đây là
  // phần còn thiếu để đóng bằng chuột. Chỉ lắng nghe khi đang mở, nên không
  // đính listener vào `window` suốt thời gian ứng dụng chạy.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target;
      // `closest` chỉ có trên `Element`; `event.target` kiểu `EventTarget` có thể là
      // `Text` hay `Document`, nên phải thu hẹp kiểu trước khi gọi (tsc bắt).
      if (!(target instanceof Element)) return;
      // Bấm trong bảng (hoặc trên chính rail) thì không đóng — chỉ bấm ra ngoài.
      if (target.closest("[data-slot='history-flyout']")) return;
      onClose();
    };
    window.addEventListener("mousedown", onPointerDown);
    return () => window.removeEventListener("mousedown", onPointerDown);
  }, [onClose, open]);

  if (!open) return null;

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())} modal={false}>
      <DialogContent
        showClose={false}
        data-slot="history-flyout"
        // `aria-label` để hộp trợ năng có tên — `DialogContent` của shadcn không
        // bắt buộc `DialogTitle`, và ở đây nhãn đã do `SessionList` in sẵn.
        aria-label={t("chat.sessions")}
        // `DialogContent` mặc định là `grid ... gap-4` cho hộp căn giữa. Ở đây cần
        // chiều cao đầy đủ và các hàng xếp từ trên xuống với `gap-0`, nên phải ghi
        // đè `grid-rows`/`gap` — không ghi đè thì tiêu đề bị `grid` chia đều chiều
        // cao và trôi xuống giữa bảng.
        className="left-0 top-0 flex h-svh w-80 max-w-none translate-x-0 translate-y-0 flex-col gap-0 overflow-hidden rounded-none border-y-0 border-l-0 p-0"
      >
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border p-3">
          {/* `sr-only` vì `SessionList` **tự** in tiêu đề "Hội thoại" ở dòng đầu.
              Giữ cả hai nhãn nhìn thấy là nói trùng; giữ cả hai cho trình đọc màn
              hình thì nó đọc hai lần. `sr-only` chừa đúng một nhãn cho người đọc. */}
          <h2 className="sr-only">{t("chat.sessions")}</h2>
          <DialogClose asChild>
            <button
              type="button"
              className="ml-auto rounded-lg p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              aria-label={t("sessions.closeSidebar")}
            >
              <XIcon className="size-4" aria-hidden="true" />
            </button>
          </DialogClose>
        </div>
        {/* `min-h-0`: `<nav>` bên trong có `flex-1 overflow-y-auto`, cần một cha
            flex có chiều cao xác định thì nó mới cuộn được thay vì tràn. */}
        <div className="flex min-h-0 flex-1 flex-col">
          <SessionList {...listProps} onNavigate={onClose} />
        </div>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Một mục trong thanh điều hướng.
 *
 * Khi thu gọn (`collapsed`), nhãn chữ **không bị xoá** mà chuyển thành
 * `sr-only`: như vậy trình đọc màn hình vẫn đọc đúng "Trò chuyện" thay vì
 * chỉ còn một biểu tượng vô danh — đây là lý do lớn nhất để có thanh rail.
 * Tooltip chỉ là **gợi ý thị giác** cho con trỏ; nó không thay thế được nhãn.
 */
function NavItem({
  to,
  label,
  icon: Icon,
  end = false,
  collapsed = false,
  onNavigate,
}: {
  to: string;
  label: string;
  icon: LucideIcon;
  end?: boolean;
  collapsed?: boolean;
  onNavigate: () => void;
}) {
  // # Vì sao `className` dạng hàm bị Radix xoá
  //
  // `NavItem` thu gọn được bọc trong `<TooltipTrigger asChild>`. Radix **clone**
  // phần tử con rồi **nối chuỗi** `className` của cả hai: `childProps.className`
  // rồi `triggerProps.className`. `className` dạng hàm của `NavLink` bị
  // `String()` hoá thành **nguồn văn bản dạng hàm**, ghép với chuỗi rỗng của
  // trigger ⇒ class trên DOM là `"({ isActive }) => ..."` chưa từng chạy.
  //
  // Hậu quả đo được: `display: block`, `padding-top: 0px`, chiều cao **16px** —
  // bằng đúng icon, tức `size-9`/`px-3`/`py-2` mất sạch. **Không có lỗi console
  // nào**; mắt cũng dễ nhầm là "cố tình bỏ padding".
  //
  // Cách sửa: class **chỉ nằm ở một chỗ mỗi nhánh**. `NavLink` nhận `className`
  // dạng **chuỗi** khi không có tooltip; khi có tooltip thì `NavLink` không đặt
  // class nào và `TooltipTrigger` giữ nó thay, để Radix nối chuỗi với chuỗi rỗng
  // — không sinh ra lần lặp.
  const isActive = useMatch({ path: to, end }) !== null;

  const itemClass = [
    "flex items-center rounded-lg text-sm transition-colors",
    // `mx-auto` không canh giữa được phần tử `display:inline` mà `<a>` mặc định
    // là; `<nav>` là `flex flex-col items-center` nên mục được cha canh giữa.
    collapsed ? "size-9 justify-center" : "w-full gap-3 px-3 py-2",
    isActive
      ? "bg-accent font-semibold text-foreground"
      : "text-muted-foreground hover:bg-background hover:text-foreground",
  ].join(" ");

  const navLink = (
    <NavLink
      to={to}
      end={end}
      onClick={onNavigate}
      title={collapsed ? label : undefined}
      // Nhánh **có tooltip** để trống: `TooltipTrigger` sẽ đặt `itemClass` thay,
      // tránh class bị Radix nối chồng hai lần.
      className={collapsed ? undefined : itemClass}
    >
      <Icon className="size-4 shrink-0" aria-hidden="true" />
      {collapsed ? <span className="sr-only">{label}</span> : <span className="truncate">{label}</span>}
    </NavLink>
  );

  // Rail 80px không chứa nổi chữ; tooltip là cách duy nhất để con trỏ biết icon
  // này là mục nào. Bọc bằng `TooltipProvider` vì Radix cần một provider.
  if (!collapsed) return navLink;
  return (
    <Tooltip>
      <TooltipTrigger asChild className={itemClass}>
        {navLink}
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
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

/** Ngôn ngữ hỗ trợ, kèm tên hiển thị **tự thân** — không dịch. */
const LANGUAGES: { code: Lang; label: string }[] = [
  { code: "vi", label: "Tiếng Việt" },
  { code: "en", label: "English" },
];

/**
 * Chọn ngôn ngữ.
 *
 * Dùng `Select` chứ không phải `DropdownMenu`: hai ngôn ngữ là hai **trạng thái**
 * song song (chọn `en` thì `vi` tự tắt), không phải những hành động. Radix cho
 * mục `role="option"` + `aria-selected`, nên trình đọc màn hình đọc "đang chọn
 * Tiếng Việt" thay vì chỉ biết có một mục tên Tiếng Việt trong menu.
 *
 * Vì sao không phải `<select>` native: mở ra là giao diện của hệ điều hành —
 * không nhận token màu của app, nền trắng đụng nền tối. Nó là thứ duy nhất
 * trong sidebar không đi qua token nên lệch chuẩn ngay nhìn thấy được.
 */
function LanguageSelect({ collapsed = false }: { collapsed?: boolean }) {
  const { t, lang, setLang } = useI18n();
  const current = LANGUAGES.find((language) => language.code === lang) ?? LANGUAGES[0];

  return (
    <Select value={lang} onValueChange={(next) => setLang(next as Lang)}>
      <SelectTrigger
        size="sm"
        // Mở rộng: `w-full` + `justify-start` để nhãn "Ngôn ngữ" và giá trị nằm
        // cùng hàng, đọc như "Ngôn ngữ: Tiếng Việt". Thu gọn: ô vuông 32px, chỉ
        // còn icon vì rail 80px không chứa nổi chữ.
        //
        // `p-0` bắt buộc: `SelectTrigger` có `py-2 px-3` **trong component**,
        // và `size-8` chỉ đặt `width`/`height` — `py-2` vẫn thắng giá trị cao 32px.
        // Đo ra `padding-top: 8px`, tức nút cao 48px trong rail 80px: tràn và lệch
        // so với hai nút bên cạnh. `cn`/`twMerge` không tự dọn vì `p-0` và
        // `py-2` thuộc **nhóm khác nhau** (`padding` toàn phần vs `padding-block`).
        //
        // `border-transparent` khi thu gọn: `SelectTrigger` có `border border-input`
        // **trong component**. Ở bản mở rộng viền đó đúng — đọc là ô nhập. Nhưng ở
        // rail 80px nó là ô vuông icon đứng cạnh nút chủ đề và nút đăng xuất (cùng
        // `variant="ghost"`, **không** viền), nên nó là nút duy nhất có viền.
        //
        // Vì sao `border-transparent` chứ không `border-0`: `cn`/`twMerge` đặt class
        // của component **trước** class ở đây, và coi `border-input` với `border-0`
        // là cùng nhóm — nên `border-0` bị **xoá mất** (test bắt được đúng chỗ
        // này), còn `border-transparent` thì thay đúng màu viền. `border-width` của
        // component vẫn còn nên kích thước không đổi.
        className={
          collapsed
            ? "size-8 shrink-0 justify-center border-transparent p-0 focus-visible:ring-2 focus-visible:ring-ring/60"
            : "w-full justify-start gap-2 px-2"
        }
        // Nhãn nói rõ đang ở ngôn ngữ nào, không bắt người dùng mở ra mới biết;
        // đây là nội dung của nút chứ không phải tooltip. `current` tra cùng
        // `LANGUAGES` với `SelectValue` nên không thể lệch với danh sách hiện ra.
        aria-label={`${t("common.language")}: ${current.label}`}
        data-slot="language-select"
      >
        <LanguagesIcon className="size-4 shrink-0 text-muted-foreground" />
        {/* Thu gọn chỉ còn icon. Mở rộng có nhãn "Ngôn ngữ" làm tiền tố, rồi tới
            `SelectValue` tự rút nhãn `<SelectItem>` đang được chọn — không phải
            viết tay lại danh sách ở đây, tránh hai nơi lệch nhau. */}
        {collapsed ? null : (
          <>
            <span className="text-muted-foreground">{t("common.language")}</span>
            <span className="text-ink-muted" aria-hidden="true">
              ·
            </span>
            <SelectValue />
          </>
        )}
      </SelectTrigger>
      <SelectContent>
        {LANGUAGES.map((language) => (
          <SelectItem key={language.code} value={language.code}>
            {language.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
