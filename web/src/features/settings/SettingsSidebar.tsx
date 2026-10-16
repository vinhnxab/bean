import { CpuIcon, type LucideIcon, XIcon } from "lucide-react";
import { NavLink } from "react-router";

import { useI18n } from "@/i18n";

/**
 * Các mục cài đặt. Giống `NAV_ITEMS` của sidebar trái: một bảng, thêm/bớt mục
 * là thêm/bớt một dòng. v1 chỉ có Providers; các mục sau (Bảo mật, Giao diện…)
 * thêm route con `/settings/<section>` rồi khai báo ở đây.
 */
const SETTINGS_SECTIONS: {
  to: string;
  labelKey: "settings.section.providers";
  descKey: "settings.section.providersDescription";
  icon: LucideIcon;
}[] = [
  {
    to: "/settings/providers",
    labelKey: "settings.section.providers",
    descKey: "settings.section.providersDescription",
    icon: CpuIcon,
  },
];

/**
 * Sidebar phải cho màn **Cài đặt**: điều hướng giữa các section, thay thế ngăn
 * kéo hội thoại (hai thứ không cùng tồn tại — đang cài đặt thì không cần danh
 * sách hội thoại chiếm cột phải).
 *
 * Cùng pattern với `RightSidebar`: desktop là cột thường trú, mobile là ngăn
 * kéo trượt từ mép phải kèm overlay bấm ra ngoài để đóng.
 */
export function SettingsSidebar({
  open,
  onClose,
  onNavigate,
}: {
  open: boolean;
  onClose: () => void;
  onNavigate: () => void;
}) {
  const { t } = useI18n();

  return (
    <>
      {open ? (
        <button
          type="button"
          aria-label={t("common.close")}
          className="fixed inset-0 z-20 bg-black/60 md:hidden"
          onClick={onClose}
        />
      ) : null}
      <aside
        data-testid="settings-sidebar"
        data-open={open || undefined}
        aria-label={t("settings.title")}
        className={`fixed inset-y-0 right-0 z-30 flex w-72 flex-col border-l border-border bg-card transition-transform duration-200 md:static md:translate-x-0 ${open ? "translate-x-0" : "translate-x-full"}`}
      >
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border p-3">
          <h2 className="text-base font-bold">{t("settings.title")}</h2>
          <button
            type="button"
            className="shrink-0 rounded-lg p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground md:hidden"
            onClick={onClose}
            aria-label={t("settings.closeSidebar")}
          >
            <XIcon className="size-5" aria-hidden="true" />
          </button>
        </div>
        <nav
          aria-label={t("settings.nav")}
          className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto p-3"
        >
          {SETTINGS_SECTIONS.map((section) => (
            <NavLink
              key={section.to}
              to={section.to}
              onClick={onNavigate}
              className={({ isActive }) =>
                [
                  "flex items-start gap-3 rounded-lg px-3 py-2 text-sm transition-colors",
                  isActive
                    ? "bg-accent font-semibold text-foreground"
                    : "text-muted-foreground hover:bg-background hover:text-foreground",
                ].join(" ")
              }
            >
              <section.icon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <span className="min-w-0">
                <span className="block truncate">{t(section.labelKey)}</span>
                <span className="mt-0.5 block text-xs text-muted-foreground">{t(section.descKey)}</span>
              </span>
            </NavLink>
          ))}
        </nav>
      </aside>
    </>
  );
}
