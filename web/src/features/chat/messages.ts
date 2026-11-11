import type { JsonValue, MessageDto } from "@/api/bindings";
import type { ConfirmState } from "@/store/chat";

export type StoredToolCall = {
  id: string;
  name: string;
  args: JsonValue;
};

/**
 * Khối ảnh đi kèm kết quả tool (M26 — `browser_screenshot`).
 *
 * `mediaType` được kiểm ở **ranh giới** và chỉ nhận đúng ba giá trị server cho
 * phép: giá trị này đi thẳng vào `data:` URI của thẻ `<img>`, nên không được tin
 * `media_type` tuỳ ý. Tập này phải khớp `ALLOWED_IMAGE_MEDIA_TYPES` ở Rust.
 */
export type StoredImage = {
  mediaType: "image/png" | "image/jpeg" | "image/webp";
  data: string;
  sha256: string;
};

const ALLOWED_IMAGE_MEDIA_TYPES = ["image/png", "image/jpeg", "image/webp"] as const;

function parseImage(value: JsonValue): StoredImage | null {
  const object = objectValue(value);
  if (!object) return null;
  const { media_type: mediaType, data, sha256 } = object;
  if (
    typeof mediaType !== "string" ||
    typeof data !== "string" ||
    typeof sha256 !== "string" ||
    !ALLOWED_IMAGE_MEDIA_TYPES.includes(mediaType as (typeof ALLOWED_IMAGE_MEDIA_TYPES)[number])
  ) {
    return null;
  }
  return { mediaType: mediaType as StoredImage["mediaType"], data, sha256 };
}

export type StoredMessage = {
  role: "user" | "assistant" | "tool";
  text: string;
  toolCalls: StoredToolCall[];
  toolCallId: string | null;
  isError: boolean;
  /** Ảnh đi kèm (chỉ ở kết quả tool trả ảnh); `null` với mọi message khác. */
  image: StoredImage | null;
};

function objectValue(value: JsonValue): Record<string, JsonValue> | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  return value;
}

function parseToolCall(value: JsonValue): StoredToolCall | null {
  const object = objectValue(value);
  if (!object || typeof object.id !== "string" || typeof object.name !== "string") return null;
  return { id: object.id, name: object.name, args: object.args ?? null };
}

/** Chuyển JsonValue từ MessageDto sang view model đã kiểm tra ở biên. */
export function parseStoredMessage(dto: MessageDto): StoredMessage | null {
  const object = objectValue(dto.message);
  if (!object) return null;
  const role = object.role;
  if (role !== "user" && role !== "assistant" && role !== "tool") return null;
  const text = typeof object.text === "string" ? object.text : "";
  const toolCalls = Array.isArray(object.tool_calls)
    ? object.tool_calls.map(parseToolCall).filter((item): item is StoredToolCall => item !== null)
    : [];
  const toolCallId = typeof object.tool_call_id === "string" ? object.tool_call_id : null;
  return {
    role,
    text,
    toolCalls,
    toolCallId,
    isError: object.is_error === true,
    image: parseImage(object.image ?? null),
  };
}

export function deduplicateMessages(messages: MessageDto[]): MessageDto[] {
  const byId = new Map<number, MessageDto>();
  for (const message of messages) byId.set(message.id, message);
  return [...byId.values()].sort((left, right) => left.seq - right.seq);
}

/**
 * Tin "quá trình làm việc" của Bean: tin assistant kèm lời gọi tool (suy nghĩ
 * trung gian) và kết quả tool. Dòng chat chỉ hiện kết quả cuối cùng nên các tin
 * này bị ẩn. Riêng kết quả tool kèm ảnh (M26 — `browser_screenshot`) vẫn hiện:
 * ảnh là nội dung người dùng cần xem, không phải nhiễu quá trình.
 */
export function isWorkMessage(message: StoredMessage): boolean {
  if (message.role === "tool") return message.image === null;
  if (message.role === "assistant") return message.toolCalls.length > 0;
  return false;
}

/** Một phần tử trong luồng chat: tin từ DB hoặc thẻ xác nhận đến trực tiếp qua WS. */
export type FlowItem = { kind: "message"; message: MessageDto } | { kind: "confirm"; confirm: ConfirmState };

/**
 * Gộp thẻ xác nhận vào dòng thời gian của tin nhắn theo thứ tự phát sinh.
 *
 * Thẻ xác nhận là trạng thái trực tiếp (WS) còn tin là lịch sử (REST): nếu render
 * thẻ luôn ở cuối danh sách thì khi `final` nạp lại lịch sử, câu trả lời cuối
 * chen lên TRƯỚC thẻ — sai dòng thời gian vì hành động xác nhận xảy ra trước
 * câu trả lời. So sánh `receivedAt` (client, epoch ms) với `created_at` (server,
 * RFC3339): thẻ chèn vào trước tin đầu tiên được tạo sau nó.
 *
 * Lệch đồng hồ client/server chỉ làm thẻ lệch vị trí về mặt hiển thị (agent cá
 * nhân: client thường cùng máy với server); sau khi tải lại trang thẻ xác nhận
 * biến mất nên sai lệch không tích tụ. Thẻ đang chờ (`receivedAt` = bây giờ) luôn
 * mới hơn mọi tin đã có → tự nằm đáy, đúng hành vi khi run đang chạy.
 */
export function mergeConfirmFlow(messages: MessageDto[], confirms: ConfirmState[]): FlowItem[] {
  const ordered = [...confirms].sort((left, right) => left.receivedAt - right.receivedAt);
  const items: FlowItem[] = [];
  let cursor = 0;
  for (const message of messages) {
    const at = Date.parse(message.created_at);
    if (!Number.isNaN(at)) {
      while (cursor < ordered.length && ordered[cursor].receivedAt <= at) {
        items.push({ kind: "confirm", confirm: ordered[cursor] });
        cursor += 1;
      }
    }
    items.push({ kind: "message", message });
  }
  for (; cursor < ordered.length; cursor += 1) {
    items.push({ kind: "confirm", confirm: ordered[cursor] });
  }
  return items;
}
