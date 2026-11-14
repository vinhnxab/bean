import type { ServerMsg } from "@/api/bindings";
import type { Translate } from "@/i18n";
import { canNotifyNow, type DesktopNotification, showDesktopNotification } from "@/lib/notify";
import { eventKey } from "@/store/chat";

/**
 * Ánh xạ sự kiện WS → thông báo hệ thống, và bộ nhớ khoá chống bắn trùng.
 *
 * # Những sự kiện nào đáng thông báo
 *
 * - `final` — trả lời xong (body là preview câu trả lời).
 * - `confirm_request` — cần người dùng duyệt hành động; bỏ qua thì run treo
 *   until hết `timeout_seconds`, nên đây là thông báo quan trọng nhất.
 * - `notification` — tin chủ động (scheduler, learning draft duyệt xong).
 * - `error` — run chết khi tab ẩn (ví dụ hết ngân sách token): không thông báo
 *   thì người dùng quay lại chỉ thấy màn hình im lặng.
 *
 * Sự kiện quá trình (`text_delta`, `tool_start`…) KHÔNG thông báo — đó là nhiễu
 * cho đúng lý do UI chat đã hiển thị chúng.
 *
 * # Vì sao cần bộ nhớ khoá riêng dù store đã dedupe
 *
 * Store khử trùng để **render** không nhân đôi; `confirm_request` và `error`
 * không sinh `RefreshEffect` nên không thể dùng nó làm tín hiệu bắn. Bộ nhớ
 * này tái dùng đúng `eventKey` của store, giữ trần 200 khoá — đủ cho các sự
 * kiện tần suất thấp (mỗi run một `final`, mỗi xác nhận một khoá).
 */

/** Trần ký tự của body thông báo — hệ điều hành tự cắt theo kiểu của nó, cắt trước cho gọn. */
const PREVIEW_LIMIT = 120;

/** Gộp whitespace và cắt ở ranh giới ký tự (không đứt surrogate pair của emoji). */
function preview(text: string, limit = PREVIEW_LIMIT): string {
  const flat = text.replace(/\s+/g, " ").trim();
  const chars = Array.from(flat);
  if (chars.length <= limit) return flat;
  return `${chars.slice(0, limit).join("")}…`;
}

export function notifyContentFor(
  message: Exclude<ServerMsg, { type: "sync" | "pong" }>,
  t: Translate,
): DesktopNotification | null {
  switch (message.type) {
    case "final":
      return {
        title: t("notify.replyTitle"),
        body: preview(message.text) || t("notify.replyBody"),
        tag: `bean:final:${message.run_id}`,
        sessionId: message.session_id,
      };
    case "confirm_request":
      return {
        title: t("notify.confirmTitle"),
        body: preview(message.prompt),
        tag: `bean:confirm:${message.confirm_id}`,
        sessionId: message.session_id,
      };
    case "notification":
      return {
        title: t("notify.pushTitle"),
        body: t("notify.pushBody"),
        tag: `bean:notification:${message.session_id}:${message.message_id}`,
        sessionId: message.session_id,
      };
    case "error":
      return {
        title: t("notify.errorTitle"),
        body: preview(message.message),
        tag: `bean:error:${message.session_id ?? "global"}:${message.run_id ?? "none"}`,
        sessionId: message.session_id,
      };
    default:
      return null;
  }
}

/**
 * Cân nhắc bắn thông báo cho một sự kiện; trả `true` nếu **đã bắn thật**.
 * `seenKeys` là mảng khoá đã bắn (do caller giữ — ref trong component), cắt bớt
 * khi vượt trần để phiên chạy lâu không phình vô hạn.
 */
export function maybeNotifyServerMessage(
  message: Exclude<ServerMsg, { type: "sync" | "pong" }>,
  t: Translate,
  seenKeys: string[],
): boolean {
  const key = eventKey(message);
  if (
    key === null ||
    (message.type !== "final" &&
      message.type !== "confirm_request" &&
      message.type !== "notification" &&
      message.type !== "error")
  ) {
    return false;
  }
  if (seenKeys.includes(key)) return false;
  seenKeys.push(key);
  if (seenKeys.length > 200) seenKeys.splice(0, seenKeys.length - 200);
  if (!canNotifyNow()) return false;
  const content = notifyContentFor(message, t);
  if (!content) return false;
  return showDesktopNotification(content);
}
