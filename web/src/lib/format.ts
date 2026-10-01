import type { JsonValue } from "@/api/bindings";

export function formatDateTime(value: string | null | undefined, locale?: string): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(date);
}

export function formatJson(value: JsonValue): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

/**
 * Ngày dạng `YYYY-MM-DD` (chốt UTC, khoá của bảng `usage_by_role`) → nhãn ngắn
 * theo ngôn ngữ, ví dụ `29 th 9` / `Sep 29`.
 *
 * # Vì sao dựng Date thủ công bằng `Date.UTC`
 *
 * `new Date("2026-09-29")` được định nghĩa là **nửa đêm UTC**, nên khi format theo
 * múi giờ local, ngày ở múi giờ âm (UTC-5) sẽ lùi thành `Sep 28` — nhãn lệch một
 * ngày so với đúng dữ liệu. Ghim `timeZone: "UTC"` loại bỏ hẳn lớp lỗi đó.
 */
export function formatDay(day: string, locale?: string): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!match) return day;
  const date = new Date(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])));
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  }).format(date);
}

export function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}
