import type { ServerMsg } from "@/api/bindings";
import type { Lang, Translate } from "@/i18n";
import type { AttentionKind } from "@/lib/attention";
import { getAttentionName, playAttention } from "@/lib/attention";
import { type DesktopNotification, showDesktopNotification } from "@/lib/notify";
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
 * # Hai kênh cho cùng một sự kiện
 *
 * Thông báo hệ thống (thẻ của hệ điều hành) và âm thanh gọi tên (`lib/attention`)
 * phục vụ hai kiểu tình huống: người nhìn máy khác (đọc được nội dung) và người
 * đeo tai nghe/rời xa màn hình (chỉ nghe được chime + lời gọi tên). Một sự kiện
 * có thể bắn cả hai; điều kiện và cách tắt của từng kênh độc lập (nút chuông/
 * nút loa trên TopNav).
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

/** Ánh xạ sự kiện WS → loại câu âm thanh. `null` với sự kiện không đáng phát tiếng. */
function attentionKindFor(message: Exclude<ServerMsg, { type: "sync" | "pong" }>): AttentionKind | null {
  switch (message.type) {
    case "final":
      return "reply";
    case "confirm_request":
      return "confirm";
    case "notification":
      return "push";
    case "error":
      return "error";
    default:
      return null;
  }
}

/**
 * Cân nhắc cảnh báo cho một sự kiện qua cả hai kênh; trả `true` nếu **có kênh
 * nào đã phát thật**.
 *
 * `seenKeys` là mảng khoá đã bắn (do caller giữ — ref trong component), dùng
 * CHUNG cho cả hai kênh để một sự kiện không kêu hai lần khi socket phát lại,
 * cắt bớt khi vượt trần để phiên chạy lâu không phình vô hạn. Điều kiện của
 * từng kênh (`canNotifyNow`/`canPlayAttentionNow`) được kiểm bên trong — hàm
 * này chỉ lo dedupe và ánh xạ nội dung.
 */
export function maybeAlertServerMessage(
  message: Exclude<ServerMsg, { type: "sync" | "pong" }>,
  t: Translate,
  lang: Lang,
  seenKeys: string[],
): boolean {
  const key = eventKey(message);
  if (key === null) return false;
  const kind = attentionKindFor(message);
  if (kind === null) return false;
  if (seenKeys.includes(key)) return false;
  seenKeys.push(key);
  if (seenKeys.length > 200) seenKeys.splice(0, seenKeys.length - 200);

  let alerted = false;
  const content = notifyContentFor(message, t);
  if (content !== null) {
    alerted = showDesktopNotification(content);
  }
  if (playAttention(kind, getAttentionName(), t, lang)) alerted = true;
  return alerted;
}
