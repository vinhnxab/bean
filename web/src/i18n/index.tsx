import { createContext, type ReactNode, useContext, useMemo, useState } from "react";

import { en } from "@/i18n/en";
import { type MessageKey, vi } from "@/i18n/vi";

/** Từ điển đã hoàn chỉnh cho một ngôn ngữ. */
export type Dictionary = Record<MessageKey, string>;

/** Ngôn ngữ hỗ trợ (agents.md mục 12.1: `vi` mặc định, `en`). */
export type Lang = "vi" | "en";

/** Tất cả từ điển, tra theo ngôn ngữ. */
export const DICTIONARIES: Record<Lang, Dictionary> = { vi, en };

/** Ngôn ngữ mặc định của ứng dụng. */
export const DEFAULT_LANG: Lang = "vi";

type I18nValue = {
  lang: Lang;
  setLang: (lang: Lang) => void;
  /**
   * Dịch một khoá. `values` nội suy `{tên}` — dùng khi cần nhúng số đếm vào
   * câu (ví dụ "Cần bạn duyệt (2)"). Không có `values` thì trả nguyên chuỗi.
   */
  t: (key: MessageKey, values?: Record<string, string | number>) => string;
};

const I18nContext = createContext<I18nValue | null>(null);

/** Thay `{tên}` bằng giá trị tương ứng; khoá thiếu thì để nguyên placeholder. */
function interpolate(template: string, values: Record<string, string | number>): string {
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in values ? String(values[name]) : match,
  );
}

/** Bọc ứng dụng để cung cấp ngôn ngữ hiện tại. */
export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLang] = useState<Lang>(DEFAULT_LANG);

  const value = useMemo<I18nValue>(
    () => ({
      lang,
      setLang,
      t: (key, values) => {
        const text = DICTIONARIES[lang][key];
        return values ? interpolate(text, values) : text;
      },
    }),
    [lang],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

/** Lấy hàm dịch và ngôn ngữ hiện tại. */
export function useI18n(): I18nValue {
  const value = useContext(I18nContext);
  if (!value) {
    // Lập trình sai (thiếu provider) — báo ngay khi phát triển, không im lặng.
    throw new Error("useI18n phải được dùng bên trong <I18nProvider>");
  }
  return value;
}
