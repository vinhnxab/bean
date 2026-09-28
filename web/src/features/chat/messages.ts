import type { JsonValue, MessageDto } from "@/api/bindings";

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

export function formatJson(value: JsonValue): string {
  try {
    return JSON.stringify(value, null, 2) ?? "null";
  } catch {
    return String(value);
  }
}

export function truncateChars(value: string, maxChars: number): string {
  const characters = [...value];
  if (characters.length <= maxChars) return value;
  const hidden = characters.length - maxChars;
  return `${characters.slice(0, maxChars).join("")}\n[+${hidden} ký tự]`;
}

export function textFromDto(dto: MessageDto): string {
  const message = dto.message;
  return typeof message === "object" &&
    message !== null &&
    !Array.isArray(message) &&
    typeof message.text === "string"
    ? message.text
    : "";
}
