import type { JsonValue, MessageDto } from "@/api/bindings";

export type StoredToolCall = {
  id: string;
  name: string;
  args: JsonValue;
};

export type StoredMessage = {
  role: "user" | "assistant" | "tool";
  text: string;
  toolCalls: StoredToolCall[];
  toolCallId: string | null;
  isError: boolean;
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
