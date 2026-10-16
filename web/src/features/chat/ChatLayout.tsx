import {
  ActivityIcon,
  BrainIcon,
  CalendarClockIcon,
  GaugeIcon,
  type LucideIcon,
  MessageSquareIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  PlugIcon,
  ScrollTextIcon,
  SettingsIcon,
  SparklesIcon,
  WrenchIcon,
  XIcon,
} from "lucide-react";
import { Suspense, useEffect, useMemo, useState } from "react";
import { NavLink, Outlet, useLocation, useMatch, useNavigate } from "react-router";

import { TopNav } from "@/components/TopNav";
import { RouteFallback } from "@/components/ui/Page";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { RightSidebar } from "@/features/chat/RightSidebar";
import { groupSessions } from "@/features/chat/sessionGroups";
import { useCreateSession, useSessions } from "@/features/sessions/queries";
import { SettingsSidebar } from "@/features/settings/SettingsSidebar";
import { useI18n } from "@/i18n";
import type { MessageKey } from "@/i18n/vi";
import { Enter } from "@/lib/anim";
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

/**
 * Khung ứng dụng: TopNav (ngôn ngữ, chủ đề, tài khoản) + sidebar trái (điều
 * hướng chính) + nội dung + sidebar phải (hội thoại).
 *
 * Sidebar phải chỉ tồn tại ở các trang **có ngữ cảnh hội thoại** — trang chủ
 * HUB không có, để sơ đồ hệ thống chiếm trọn chỗ.
 */
export function ChatLayout() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const location = useLocation();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [collapsed, toggleCollapsed] = useSidebarCollapsed();
  // Ngăn kéo hội thoại (bên phải) chỉ có ý nghĩa trên mobile — desktop sidebar
  // luôn nằm sẵn. Mặc định đóng: mở trang trên desktop không bị "nhớ" trạng thái
  // ngăn kéo đã mở ở lần dùng mobile trước.
  const [historyOpen, setHistoryOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [archived, setArchived] = useState(false);
  const sessions = useSessions({ q: search, archived });
  // Trang chủ (HUB) không cần danh sách hội thoại: hội thoại là ngữ cảnh của
  // trang chat, đặt cạnh sơ đồ hệ thống chỉ làm loãng và tốn nửa màn hình.
  // Màn Cài đặt thì ngược lại: cột phải là **nav cài đặt** (SettingsSidebar),
  // hai ngăn kéo không cùng tồn tại.
  const isSettings = location.pathname.startsWith("/settings");
  const showSessions = !isSettings && location.pathname !== "/";
  // Cơ sở route cho hoạt ảnh chuyển màn: đổi `/sessions/1 → /sessions/2` KHÔNG
  // phải là "chuyển trang" (cùng khung chat, chỉ đổi ngữ cảnh) — remount cả khung
  // sẽ phá cuộn và trạng thái phiên đang chạy, nên gộp hai pathname về một khoá.
  const routeBase = location.pathname.replace(/\/sessions\/\d+$/, "/sessions");
  const createSession = useCreateSession();
  const [actionError, setActionError] = useState(false);
  // Gom nhóm theo mốc thời gian. `useMemo` vì `Date.now()` trả về mỗi lần
  // render — nếu không, danh sách bị dựng lại liên tục và nhảy nhóm khi
  // người dùng đang đọc (tin chủ động hay về giữa lúc họ xem danh sách).
  const groups = useMemo(
    () => groupSessions(sessions.data?.sessions ?? [], new Date()),
    [sessions.data?.sessions],
  );

  // Rời các trang chat và cài đặt (về HUB) thì đóng ngăn kéo phải: khi quay lại,
  // mở đúng trạng thái người dùng vừa định hướng xong thay vì bắt họ đóng lại.
  useEffect(() => {
    if (!showSessions && !isSettings) setHistoryOpen(false);
  }, [showSessions, isSettings]);

  async function newSession() {
    if (createSession.isPending) return;
    setActionError(false);
    try {
      const session = await createSession.mutateAsync({ title: null });
      setHistoryOpen(false);
      setMobileOpen(false);
      navigate(`/sessions/${session.id}`);
    } catch {
      setActionError(true);
    }
  }

  function deleted(id: number) {
    setHistoryOpen(false);
    setMobileOpen(false);
    if (location.pathname === `/sessions/${id}`) navigate("/");
  }

  return (
    // Khung chia hai hàng: TopNav trên cùng, bên dưới là hàng ba cột
    // (điều hướng | nội dung | hội thoại). `h-svh overflow-hidden` giữ khung
    // cao đúng một màn hình; từng cột tự cuộn bên trong.
    <div className="flex h-svh flex-col overflow-hidden bg-background text-foreground">
      <TopNav
        onMenuOpen={() => setMobileOpen(true)}
        onHistoryOpen={showSessions || isSettings ? () => setHistoryOpen(true) : undefined}
        rightDrawerLabel={isSettings ? t("settings.openInSidebar") : undefined}
      />
      <div className="flex min-h-0 flex-1 overflow-hidden">
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
          {/* Nút đóng ngăn kéo ở đầu sidebar: chỉ có ở `< md`. Brand đã nằm ở
              TopNav nên đầu ngăn kéo chỉ còn nút đóng, căn phải. */}
          <div className="flex items-center justify-end border-b border-border p-3 md:hidden">
            <button
              type="button"
              className="shrink-0 rounded-lg p-2 text-muted-foreground hover:bg-accent hover:text-foreground"
              onClick={() => setMobileOpen(false)}
              aria-label={t("common.close")}
            >
              <XIcon className="size-5" aria-hidden="true" />
            </button>
          </div>
          {/* Nút thu gọn rail: chỉ có ở desktop. Khi thu gọn, nút được **nền đậm
              hơn** (`bg-accent`) — thứ duy nhất đưa sidebar trở lại, mà trên nền
              tối một icon `text-muted-foreground` dễ bị bỏ qua. `justify-center`
              khi thu gọn để giữ cảm giác "cột icon". */}
          <div
            className={`hidden items-center border-b border-border p-3 md:flex ${collapsed ? "justify-center" : "justify-end"}`}
          >
            <button
              type="button"
              onClick={toggleCollapsed}
              // Nhãn mô tả **hành động sắp làm**, không phải trạng thái hiện tại.
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
              // trải hết chiều ngang.
              //
              // `gap-1` thay cho `space-y-1` cũ: `gap` là thuộc tính của cha,
              // không dựa vào display của con, nên ổn định ở cả hai trạng thái.
              className={`flex flex-col gap-1 md:flex-1 ${collapsed ? "items-center p-2" : "p-3"}`}
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
            {/* Footer điều hướng: các mục "hệ thống" nằm tách khỏi điều hướng
                chính, dưới cùng của sidebar. Cài đặt ít dùng hơn chat nên không
                chiếm chỗ giữa danh sách; sát đáy thì luôn biết tìm ở đâu. */}
            <div className={`mt-auto border-t border-border ${collapsed ? "p-2" : "p-3"}`}>
              <NavItem
                to="/settings"
                label={t("nav.settings")}
                icon={SettingsIcon}
                collapsed={collapsed}
                onNavigate={() => setMobileOpen(false)}
              />
            </div>
          </TooltipProvider>
        </aside>
        <main className="flex min-w-0 flex-1 flex-col overflow-y-auto">
          {/* `Suspense` đặt quanh `Outlet` chứ không quanh cả `ChatLayout` (ở `App.tsx`):
              khi đang tải chunk của màn, chỉ vùng nội dung được thay bằng skeleton —
              sidebar vẫn đứng yên. Bọc ở ngoài sẽ làm cả khung giao diện biến mất rồi
              nhảy lại, nhấp nháy rõ rệt khi bấm chuyển màn trên mạng chậm. */}
          <Suspense fallback={<RouteFallback />}>
            {/* Chuyển màn: nội dung mới trượt nhẹ lên để mắt theo được quãng đường
                điều hướng — còn đổi phiên trong cùng `/sessions/:id` thì không
                (cùng `routeBase`, không remount): khung chat đã có nhịp riêng. */}
            <Enter key={routeBase} kind="rise" className="flex min-h-0 flex-1 flex-col">
              <Outlet />
            </Enter>
          </Suspense>
        </main>
        {isSettings ? (
          <SettingsSidebar
            open={historyOpen}
            onClose={() => setHistoryOpen(false)}
            onNavigate={() => setMobileOpen(false)}
          />
        ) : showSessions ? (
          <RightSidebar
            open={historyOpen}
            onClose={() => setHistoryOpen(false)}
            onNavigate={() => setMobileOpen(false)}
            onDeleted={deleted}
            onNewSession={() => void newSession()}
            creating={createSession.isPending}
            actionError={actionError}
            search={search}
            onSearchChange={setSearch}
            archived={archived}
            onArchivedChange={setArchived}
            sessions={sessions}
            groups={groups}
          />
        ) : null}
      </div>
    </div>
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
