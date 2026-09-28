import { describe, expect, it } from "vitest";

import { DICTIONARIES } from "@/i18n";
import { en } from "@/i18n/en";
import { vi } from "@/i18n/vi";

/**
 * agents.md mục 20: "i18n đủ khoá cho `vi` và `en`" — test này bắt lỗi thiếu khoá
 * hoặc khoá thừa ngay khi thêm chuỗi mới.
 */
describe("i18n", () => {
  it("vi và en có đúng cùng tập khoá", () => {
    const viKeys = Object.keys(vi).sort();
    const enKeys = Object.keys(en).sort();

    expect(enKeys).toEqual(viKeys);
  });

  it("không có bản dịch rỗng", () => {
    for (const [lang, dictionary] of Object.entries(DICTIONARIES)) {
      for (const [key, value] of Object.entries(dictionary)) {
        expect(value.trim(), `${lang}.${key} rỗng`).not.toBe("");
      }
    }
  });

  it("mặc định là tiếng Việt", () => {
    expect(DICTIONARIES.vi["app.title"]).toBe("Bean");
  });
});
