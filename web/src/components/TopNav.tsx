import { LogOutIcon, MenuIcon, PanelRightIcon } from "lucide-react";
import { useNavigate } from "react-router";

import { BeanAvatar } from "@/components/brand/BeanAvatar";
import { ThemeToggle } from "@/components/ThemeToggle";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useLogout } from "@/features/auth/queries";
import { type Lang, useI18n } from "@/i18n";

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
 * không nhận token màu của app, nền trắng đụng nền tối.
 */
function LanguageSelect() {
  const { t, lang, setLang } = useI18n();
  const current = LANGUAGES.find((language) => language.code === lang) ?? LANGUAGES[0];

  return (
    <Select value={lang} onValueChange={(next) => setLang(next as Lang)}>
      <SelectTrigger
        size="sm"
        className="h-8 w-auto gap-2 px-2 text-sm"
        // Nhãn nói rõ đang ở ngôn ngữ nào, không bắt người dùng mở ra mới biết;
        // đây là nội dung của nút chứ không phải tooltip. `current` tra cùng
        // `LANGUAGES` với `SelectValue` nên không thể lệch với danh sách hiện ra.
        aria-label={`${t("common.language")}: ${current.label}`}
        data-slot="language-select"
      >
        <SelectValue />
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

/**
 * Thanh điều hướng trên cùng — lớp kính **nổi** trên cột giữa: desktop nav nằm
 * gọn giữa hai sidebar (kiểu iPadOS), mobile trọn bề ngang. Nội dung cột giữa
 * cuộn chui xuống phía dưới và bị backdrop-blur làm mờ (chất kính chỉ hiện ra
 * khi có gì đó trôi phía sau — đây là lý do nav là overlay).
 *
 * `md:z-40` thắng `z-20`/`z-30` của hai sidebar: chúng là flex item nên
 * z-index vẫn có tác dụng kể cả khi `md:static`. Dưới `md` giữ `z-10` để ngăn
 * kéo trượt ĐÈ lên nav (hành vi sheet iOS).
 */
export function TopNav({
  onMenuOpen,
  onHistoryOpen,
  rightDrawerLabel,
  sidebarCollapsed,
  hasRightColumn,
}: {
  /** Mở ngăn kéo điều hướng (bên trái) — chỉ có trên mobile. */
  onMenuOpen: () => void;
  /** Mở ngăn kéo cột phải (hội thoại hoặc cài đặt) — mobile, và không có ở trang HUB. */
  onHistoryOpen?: () => void;
  /** Nhãn nút mở ngăn kéo cột phải, đổi theo ngữ cảnh trang hiện tại. */
  rightDrawerLabel?: string;
  /** Rail trái đang thu gọn — nav co mép trái theo. Giá trị `md:left-20`/
      `md:left-72` phải khớp đúng `md:w-20`/`md:w-72` của aside trong ChatLayout. */
  sidebarCollapsed: boolean;
  /** Cột phải có tồn tại không (mọi trang trừ HUB) — nav co mép phải lại. */
  hasRightColumn: boolean;
}) {
  const { t } = useI18n();
  const navigate = useNavigate();
  const logout = useLogout();

  async function signOut() {
    await logout.mutateAsync();
    navigate("/login", { replace: true });
  }

  return (
    <header
      className={[
        // Mobile: trọn bề ngang (hai sidebar là ngăn kéo trượt đè lên).
        // Desktop: nằm gọn trong cột giữa, hai sidebar trải hết chiều cao
        // hai bên. `inset-x-0` bị `md:left-*/md:right-*` ghi đè ở ≥ md.
        "glass absolute inset-x-0 top-0 z-10 md:z-40 flex h-14 items-center justify-between gap-2 border-b border-border px-3",
        // Mép nav trượt mượt cùng nhịp `duration-200` của aside khi gập/mở rail.
        "transition-[left,right] duration-200",
        sidebarCollapsed ? "md:left-20" : "md:left-72",
        hasRightColumn ? "md:right-72" : "md:right-0",
      ].join(" ")}
    >
      <div className="flex min-w-0 items-center gap-1">
        <button
          type="button"
          className="shrink-0 rounded-lg p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground md:hidden"
          onClick={onMenuOpen}
          aria-label={t("common.menu")}
        >
          <MenuIcon className="size-5" aria-hidden="true" />
        </button>
        {onHistoryOpen ? (
          <button
            type="button"
            className="shrink-0 rounded-lg p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground md:hidden"
            onClick={onHistoryOpen}
            aria-label={rightDrawerLabel ?? t("sessions.openInSidebar")}
          >
            <PanelRightIcon className="size-5" aria-hidden="true" />
          </button>
        ) : null}
        <BeanAvatar size={32} title={t("app.title")} />
        <div className="ml-2 hidden min-w-0 sm:block">
          <p className="truncate text-sm font-bold">{t("app.title")}</p>
          <p className="truncate text-xs text-muted-foreground">{t("app.tagline")}</p>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1">
        <LanguageSelect />
        {/* Icon trơn: top nav là hàng công cụ toàn icon, nhãn chữ sẽ phá nhịp —
            `aria-label` của `ThemeToggle` vẫn nói đủ "Chuyển sang giao diện sáng". */}
        <ThemeToggle className="size-8" />
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex size-9 shrink-0 items-center justify-center rounded-lg transition-colors hover:bg-accent"
              aria-label={t("common.account")}
            >
              {/* Ảnh đại diện: ứng dụng một người dùng, avatar chính là Bean. */}
              <BeanAvatar size={26} />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => void signOut()} disabled={logout.isPending} className="gap-2">
              <LogOutIcon className="size-4" aria-hidden="true" />
              {logout.isPending ? t("common.loading") : t("chat.logout")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </header>
  );
}
