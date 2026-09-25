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
  http.get("/api/tasks", () => HttpResponse.json({ tasks: [] })),
  http.get("/api/audit", () => HttpResponse.json({ entries: [] })),
);
