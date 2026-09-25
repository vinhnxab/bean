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
);
