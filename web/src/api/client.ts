import type {
  AgentListResponse,
  AuditListResponse,
  AuthMeResponse,
  CreateSessionRequest,
  DeleteResponse,
  LlmConfigResponse,
  LlmModelUpdateRequest,
  LlmModelUpdateResponse,
  LlmProviderCreateRequest,
  LlmProviderSwitchRequest,
  LlmProviderUpdateRequest,
  LoginRequest,
  LoginResponse,
  LogoutResponse,
  McpServerListResponse,
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
  SystemResponse,
  TaskDto,
  TaskListResponse,
  TaskRequest,
  TaskUpdateRequest,
  ToolListResponse,
  UpdateSessionRequest,
  UsageListResponse,
  VoiceStatusResponse,
  VoiceSynthesizeRequest,
  VoiceSynthesizeResponse,
  VoiceTranscribeRequest,
  VoiceTranscribeResponse,
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

  /**
   * Danh sách agent cho HUB. Server đã lọc theo RBAC **trước khi** trả, nên
   * response chỉ chứa domain mà vai trò của người gọi được phép thấy — client
   * không cần (và không được) lọc lần nữa bằng cách ẩn/hiện.
   */
  listAgents: (signal?: AbortSignal) => requestJson<AgentListResponse>("/api/agents", { signal }),

  /**
   * Tool đang chạy trong agent, **đã lọc theo RBAC ở server**.
   *
   * `total` là tổng registry, `tools.length` là phần người gọi được thấy — hai số
   * khác nhau là cố ý để UI nói được "27 tool đang chạy, bạn thấy 12" thay vì để
   * người dùng tưởng Bean chỉ có 12 tool.
   */
  listTools: (signal?: AbortSignal) => requestJson<ToolListResponse>("/api/tools", { signal }),

  /**
   * MCP server đã khai báo + trạng thái suy ra từ registry.
   *
   * API **không** trả `env` của server: giá trị biến môi trường là secret, và tên
   * biến cũng gợi ý loại credential đang dùng.
   */
  listMcpServers: (signal?: AbortSignal) => requestJson<McpServerListResponse>("/api/mcp", { signal }),

  /** Token đã dùng theo từng ngày (server clamp 1–31 ngày). */
  usage: (days = 14, signal?: AbortSignal) =>
    requestJson<UsageListResponse>(withQuery("/api/usage", { days }), { signal }),

  /**
   * Cấu hình LLM runtime **đã khử secret** — API key, bot token và MCP `env` không bao
   * giờ đi qua endpoint này; chỉ có *tên* biến môi trường của API key.
   */
  system: (signal?: AbortSignal) => requestJson<SystemResponse>("/api/system", { signal }),

  /**
   * Cấu hình LLM runtime, cùng nguồn sự thật với lệnh `/model` (đổi runtime được).
   * Đã khử secret: chỉ trả *tên* biến môi trường và cờ `key_configured`.
   */
  llmConfig: (signal?: AbortSignal) => requestJson<LlmConfigResponse>("/api/config/llm", { signal }),

  /**
   * Đổi model runtime; model phải nằm trong `allowed_models` mà server đã trả.
   * Server trả 422 + `model_not_allowed` nếu model không được phép.
   */
  updateLlmModel: (request: LlmModelUpdateRequest) =>
    requestJson<LlmModelUpdateResponse>("/api/config/llm/model", { method: "PUT", body: request }),

  /**
   * Hot-swap provider theo profile đã khai báo trong `[[llm.providers]]`. Trả về
   * snapshot `LlmConfigResponse` sau khi switch (provider mới dùng cho lượt kế tiếp).
   * Lỗi 422: `profile_not_found` / `model_not_allowed` / `provider_build_failed`
   * (thiếu API key trong môi trường — key không bao giờ gửi qua body).
   */
  switchLlmProvider: (request: LlmProviderSwitchRequest) =>
    requestJson<LlmConfigResponse>("/api/config/llm/provider", { method: "PUT", body: request }),

  /**
   * Thêm profile vào `[[llm.providers]]` của bean.toml (CRUD từ UI). Trả snapshot
   * `LlmConfigResponse` sau khi thêm; 201 khi thành công.
   * Lỗi: 409 `profile_exists`, 422 `profile_invalid` / `invalid_provider`,
   * 503 `config_readonly` (server không biết đường bean.toml).
   */
  createLlmProvider: (request: LlmProviderCreateRequest) =>
    requestJson<LlmConfigResponse>("/api/config/llm/providers", { method: "POST", body: request }),

  /**
   * Sửa profile theo tên trên đường dẫn (tên không đổi được). Nếu profile đang
   * active, server rebuild provider ngay — lỗi thiếu key trả 422
   * `provider_build_failed` và giữ nguyên trạng thái cũ.
   */
  updateLlmProviderProfile: (name: string, request: LlmProviderUpdateRequest) =>
    requestJson<LlmConfigResponse>(`/api/config/llm/providers/${encodeURIComponent(name)}`, {
      method: "PUT",
      body: request,
    }),

  /** Xoá profile; 409 `profile_active` nếu profile đang active. */
  deleteLlmProviderProfile: (name: string) =>
    requestJson<LlmConfigResponse>(`/api/config/llm/providers/${encodeURIComponent(name)}`, {
      method: "DELETE",
      body: {},
    }),

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

  /**
   * Trang audit mới nhất (cũ hơn `before`).
   *
   * `limit` mặc định khớp `AUDIT_PAGE_SIZE` của màn Audit; widget HUB truyền số
   * nhỏ hơn vì nó chỉ hiện 5 dòng — tải 25 dòng rồi cắt còn 5 là tải thừa mà
   * người dùng không đợi được. Server clamp tối đa 500.
   */
  listAudit: (before: number | null = null, signal?: AbortSignal, limit = 25) =>
    requestJson<AuditListResponse>(withQuery("/api/audit", { before, limit }), { signal }),

  status: (signal?: AbortSignal) => requestJson<StatusResponse>("/api/status", { signal }),

  /**
   * Trạng thái voice (M27). UI gọi một lần để quyết định có hiện nút mic.
   * Voice tắt vẫn trả 200 với `enabled: false` — không phải lỗi.
   */
  voiceStatus: (signal?: AbortSignal) => requestJson<VoiceStatusResponse>("/api/voice", { signal }),

  /** STT: audio base64 trong JSON (xem `VoiceTranscribeRequest`). */
  transcribe: (request: VoiceTranscribeRequest) =>
    requestJson<VoiceTranscribeResponse>("/api/voice/transcribe", { method: "POST", body: request }),

  /** TTS: trả audio base64 để client dựng `Blob` và phát. */
  synthesize: (request: VoiceSynthesizeRequest) =>
    requestJson<VoiceSynthesizeResponse>("/api/voice/synthesize", { method: "POST", body: request }),
};
