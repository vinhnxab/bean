import { HttpResponse, http } from "msw";
import { setupServer } from "msw/node";

export const testServer = setupServer(
  http.get("/api/auth/me", () => HttpResponse.json({ user_id: "web:admin" })),
  http.get("/api/sessions", () => HttpResponse.json({ sessions: [] })),
  http.get("/api/sessions/:id/messages", () => HttpResponse.json({ messages: [] })),
  http.post("/api/sessions", () =>
    HttpResponse.json(
      {
        id: 1,
        channel: "web",
        chat_id: "web:admin",
        user_id: "web:admin",
        title: "",
        archived: false,
        created_at: new Date(0).toISOString(),
        updated_at: new Date(0).toISOString(),
      },
      { status: 201 },
    ),
  ),
  http.get("/api/status", () =>
    HttpResponse.json({
      version: "0.1.0",
      model: "fake-model",
      max_steps: 25,
      daily_token_budget: 1000,
      tokens_used: 0,
      uptime_seconds: 0,
      channels: ["web"],
    }),
  ),
  http.get("/api/memory/files/:name", ({ params }) =>
    HttpResponse.json({ name: String(params.name), content: "" }),
  ),
  http.get("/api/memories", () => HttpResponse.json({ memories: [] })),
  http.get("/api/skills", () => HttpResponse.json({ skills: [] })),
  http.get("/api/skills/drafts", () => HttpResponse.json({ drafts: [] })),
  http.get("/api/tasks", () => HttpResponse.json({ tasks: [] })),
  http.get("/api/audit", () => HttpResponse.json({ entries: [] })),
  // `/api/agents` mặc định trả danh sách rỗng: phần lớn test chỉ cần "không lỗi".
  // Test HUB dùng `testServer.use(...)` để ghi đè theo từng kịch bản.
  http.get("/api/agents", () => HttpResponse.json({ agents: [], viewer_role: "admin" })),
);

/**
 * Dữ liệu agent cho test — phản ánh đúng hệ trong `bean.example.toml`:
 * Manager điều phối, QA review (four-eyes), Security-scan có đường cảnh báo riêng.
 */
export const FULL_AGENTS = [
  { role: "developer", status: "working", summary: "đang thực hiện lượt", risks: [], relation: "manages" },
  { role: "qa", status: "idle", summary: "không có việc nào đang chạy", risks: [], relation: "reviews" },
  {
    role: "marketing",
    status: "awaiting_you",
    summary: "1 hành động đang chờ bạn duyệt",
    risks: ["marketing_publish"],
    relation: "manages",
  },
  {
    role: "security-scan",
    status: "idle",
    summary: "không có việc nào đang chạy",
    risks: [],
    relation: "alerts_directly",
  },
] as const;

/** Fixture `/api/agents` cho một tập agent tuỳ ý. */
export function agentsHandler(agents: unknown[], viewerRole = "admin") {
  return http.get("/api/agents", () => HttpResponse.json({ agents, viewer_role: viewerRole }));
}
