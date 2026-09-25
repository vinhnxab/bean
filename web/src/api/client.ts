import type {
  AuthMeResponse,
  CreateSessionRequest,
  LoginRequest,
  LoginResponse,
  LogoutResponse,
  MessageDto,
  MessageListResponse,
  SessionDto,
  SessionListResponse,
} from "@/api/bindings";
import type { ApiError as ApiErrorDto } from "@/api/generated/ApiError";

/** Lỗi HTTP có mã ổn định từ API. */
export class ApiRequestError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiRequestError";
  }
}

type UnauthorizedHandler = () => void;
let unauthorizedHandler: UnauthorizedHandler | null = null;

/** Client có thể thay callback 401 (App dùng callback này để điều hướng). */
export function setUnauthorizedHandler(handler: UnauthorizedHandler | null): void {
  unauthorizedHandler = handler;
}

type RequestOptions = {
  method?: "GET" | "POST" | "PATCH" | "DELETE";
  body?: unknown;
  signal?: AbortSignal;
  /** Đăng nhập sai cần hiện lỗi tại form, không tự điều hướng. */
  redirectOnUnauthorized?: boolean;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseApiError(value: unknown, status: number): ApiRequestError {
  if (isRecord(value) && typeof value.code === "string" && typeof value.message === "string") {
    const dto: ApiErrorDto = { code: value.code, message: value.message };
    return new ApiRequestError(status, dto.code, dto.message);
  }
  return new ApiRequestError(status, "http_error", `HTTP ${status}`);
}

async function requestJson<T>(
  path: string,
  { method = "GET", body, signal, redirectOnUnauthorized = true }: RequestOptions = {},
): Promise<T> {
  const headers = new Headers({ Accept: "application/json" });
  let requestBody: string | undefined;
  if (body !== undefined) {
    headers.set("Content-Type", "application/json");
    requestBody = JSON.stringify(body);
  }

  const response = await fetch(path, {
    method,
    credentials: "same-origin",
    headers,
    body: requestBody,
    signal,
  });

  if (response.status === 401 && redirectOnUnauthorized) {
    unauthorizedHandler?.();
  }
  if (!response.ok) {
    let payload: unknown = null;
    try {
      payload = await response.json();
    } catch {
      // API luôn trả JSON, nhưng lỗi proxy/network vẫn phải thành lỗi có kiểu.
    }
    throw parseApiError(payload, response.status);
  }
  if (response.status === 204) {
    return undefined as T;
  }
  try {
    return (await response.json()) as T;
  } catch {
    throw new ApiRequestError(response.status, "invalid_response", "Phản hồi API không hợp lệ");
  }
}

function withQuery(path: string, query: Record<string, string | number | boolean | null>): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value !== null) params.set(key, String(value));
  }
  const encoded = params.toString();
  return encoded ? `${path}?${encoded}` : path;
}

export const api = {
  me: (signal?: AbortSignal) => requestJson<AuthMeResponse>("/api/auth/me", { signal }),

  login: (request: LoginRequest) =>
    requestJson<LoginResponse>("/api/auth/login", {
      method: "POST",
      body: request,
      redirectOnUnauthorized: false,
    }),

  logout: () => requestJson<LogoutResponse>("/api/auth/logout", { method: "POST", body: {} }),

  listSessions: (signal?: AbortSignal) =>
    requestJson<SessionListResponse>(withQuery("/api/sessions", { archived: false, limit: 100 }), { signal }),

  createSession: (request: CreateSessionRequest) =>
    requestJson<SessionDto>("/api/sessions", { method: "POST", body: request }),

  listMessages: (sessionId: number, before: number | null, signal?: AbortSignal) =>
    requestJson<MessageListResponse>(
      withQuery(`/api/sessions/${sessionId}/messages`, { before, limit: 30 }),
      { signal },
    ),

  getMessage: (messageId: number, signal?: AbortSignal) =>
    requestJson<MessageDto>(`/api/messages/${messageId}`, { signal }),
};
