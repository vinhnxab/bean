import type {
  AuditListResponse,
  AuthMeResponse,
  CreateSessionRequest,
  DeleteResponse,
  LoginRequest,
  LoginResponse,
  LogoutResponse,
  MemoryFileRequest,
  MemoryFileResponse,
  MemoryListResponse,
  MessageDto,
  MessageListResponse,
  OkResponse,
  SessionDto,
  SessionListResponse,
  SessionQuery,
  SkillDetail,
  SkillDraftDecisionRequest,
  SkillDraftDecisionResponse,
  SkillDraftListResponse,
  SkillListResponse,
  StatusResponse,
  TaskDto,
  TaskListResponse,
  TaskRequest,
  TaskUpdateRequest,
  UpdateSessionRequest,
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
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
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
  if (response.status === 204) return undefined as T;
  try {
    return (await response.json()) as T;
  } catch {
    throw new ApiRequestError(response.status, "invalid_response", "Phản hồi API không hợp lệ");
  }
}

function withQuery(
  path: string,
  query: Record<string, string | number | boolean | null | undefined>,
): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value !== null && value !== undefined) params.set(key, String(value));
  }
  const encoded = params.toString();
  return encoded ? `${path}?${encoded}` : path;
}

export type SessionListOptions = Partial<SessionQuery>;

export const api = {
  me: (signal?: AbortSignal) => requestJson<AuthMeResponse>("/api/auth/me", { signal }),

  login: (request: LoginRequest) =>
    requestJson<LoginResponse>("/api/auth/login", {
      method: "POST",
      body: request,
      redirectOnUnauthorized: false,
    }),

  logout: () => requestJson<LogoutResponse>("/api/auth/logout", { method: "POST", body: {} }),

  listSessions: (query: SessionListOptions = {}, signal?: AbortSignal) =>
    requestJson<SessionListResponse>(
      withQuery("/api/sessions", {
        q: query.q ?? null,
        archived: query.archived ?? false,
        limit: query.limit ?? 100,
      }),
      { signal },
    ),

  createSession: (request: CreateSessionRequest) =>
    requestJson<SessionDto>("/api/sessions", { method: "POST", body: request }),

  updateSession: (id: number, request: UpdateSessionRequest) =>
    requestJson<OkResponse>(`/api/sessions/${id}`, { method: "PATCH", body: request }),

  deleteSession: (id: number) =>
    requestJson<DeleteResponse>(`/api/sessions/${id}`, { method: "DELETE", body: {} }),

  listMessages: (sessionId: number, before: number | null, signal?: AbortSignal) =>
    requestJson<MessageListResponse>(
      withQuery(`/api/sessions/${sessionId}/messages`, { before, limit: 30 }),
      { signal },
    ),

  getMessage: (messageId: number, signal?: AbortSignal) =>
    requestJson<MessageDto>(`/api/messages/${messageId}`, { signal }),

  getMemoryFile: (name: "MEMORY" | "USER", signal?: AbortSignal) =>
    requestJson<MemoryFileResponse>(`/api/memory/files/${name}`, { signal }),

  putMemoryFile: (name: "MEMORY" | "USER", request: MemoryFileRequest) =>
    requestJson<MemoryFileResponse>(`/api/memory/files/${name}`, {
      method: "PUT",
      body: request,
    }),

  listMemories: (query = "", signal?: AbortSignal) =>
    requestJson<MemoryListResponse>(withQuery("/api/memories", { q: query, limit: 100 }), { signal }),

  deleteMemory: (id: number) =>
    requestJson<DeleteResponse>(`/api/memories/${id}`, { method: "DELETE", body: {} }),

  listSkills: (signal?: AbortSignal) => requestJson<SkillListResponse>("/api/skills", { signal }),

  getSkill: (name: string, signal?: AbortSignal) =>
    requestJson<SkillDetail>(`/api/skills/${encodeURIComponent(name)}`, { signal }),

  listSkillDrafts: (signal?: AbortSignal) =>
    requestJson<SkillDraftListResponse>("/api/skills/drafts", { signal }),

  approveSkillDraft: (id: string, request: SkillDraftDecisionRequest) =>
    requestJson<SkillDraftDecisionResponse>(`/api/skills/drafts/${encodeURIComponent(id)}/approve`, {
      method: "POST",
      body: request,
    }),

  rejectSkillDraft: (id: string, request: SkillDraftDecisionRequest) =>
    requestJson<SkillDraftDecisionResponse>(`/api/skills/drafts/${encodeURIComponent(id)}/reject`, {
      method: "POST",
      body: request,
    }),

  listTasks: (signal?: AbortSignal) => requestJson<TaskListResponse>("/api/tasks", { signal }),

  createTask: (request: TaskRequest) => requestJson<TaskDto>("/api/tasks", { method: "POST", body: request }),

  updateTask: (id: number, request: TaskUpdateRequest) =>
    requestJson<TaskDto>(`/api/tasks/${id}`, { method: "PATCH", body: request }),

  deleteTask: (id: number) => requestJson<DeleteResponse>(`/api/tasks/${id}`, { method: "DELETE", body: {} }),

  listAudit: (before: number | null = null, signal?: AbortSignal) =>
    requestJson<AuditListResponse>(withQuery("/api/audit", { before, limit: 25 }), { signal }),

  status: (signal?: AbortSignal) => requestJson<StatusResponse>("/api/status", { signal }),
};
