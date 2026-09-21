import { useI18n } from "@/i18n";

/**
 * Trang trống của M1 (agents.md mục 21).
 *
 * M10 sẽ thay bằng `react-router` với route bảo vệ + màn đăng nhập + màn chat;
 * M11 thêm các màn quản lý. Ở M1 chỉ cần chứng minh: Vite + React + TS strict +
 * Tailwind + i18n chạy được, font tự host, không tài nguyên ngoài.
 */
export default function App() {
  const { t } = useI18n();

  return (
    <main className="flex min-h-svh flex-col items-center justify-center gap-3 p-6 text-center">
      <h1 className="font-semibold text-3xl tracking-tight">{t("app.title")}</h1>
      <p className="text-slate-600 dark:text-slate-400">{t("app.tagline")}</p>
      <p className="max-w-md text-slate-500 text-sm">{t("common.m1Placeholder")}</p>
    </main>
  );
}
