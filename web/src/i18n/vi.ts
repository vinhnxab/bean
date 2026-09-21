/**
 * Từ điển tiếng Việt (mặc định). Xem `docs/decisions.md` D3.12: dùng từ điển tự viết,
 * không thêm react-i18next.
 *
 * Quy ước khoá: `<màn hình>.<phần tử>`. Mọi khoá phải có mặt trong cả `vi` và `en`
 * (có test kiểm tra ở `src/i18n/i18n.test.ts`).
 */
export const vi = {
  "app.title": "BeanAgent",
  "app.tagline": "Trợ lý AI cá nhân, self-hosted",
  "common.loading": "Đang tải…",
  "common.m1Placeholder": "Khung giao diện M1 — màn hình chat sẽ có ở M10.",
} as const;

/** Tập khoá hợp lệ của từ điển (nguồn sự thật là `vi`). */
export type MessageKey = keyof typeof vi;
