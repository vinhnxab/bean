import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";

import type { SessionDto } from "@/api/bindings";
import { ChatLayout } from "@/features/chat/ChatLayout";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const session: SessionDto = {
  id: 4,
  channel: "web",
  chat_id: "web:admin",
  user_id: "web:admin",
  title: "Ke hoach hom nay",
  archived: false,
  created_at: "2026-09-26T00:00:00Z",
  updated_at: "2026-09-26T00:00:00Z",
};

function renderSidebar() {
  const user = userEvent.setup();
  renderManagement(
    <Routes>
      <Route element={<ChatLayout />}>
        <Route path="/" element={<div>hub</div>} />
        <Route path="/sessions/:sessionId" element={<div>chat</div>} />
      </Route>
    </Routes>,
    ["/sessions/4"],
  );
  return user;
}

describe("ChatLayout sidebar", () => {
  it("tim hoi thoai theo tieu de va bao ro khi khong co ket qua", async () => {
    testServer.use(
      http.get("/api/sessions", ({ request }) => {
        const query = new URL(request.url).searchParams.get("q");
        return HttpResponse.json({ sessions: query ? [] : [session] });
      }),
    );
    const user = renderSidebar();
    await screen.findByText("Ke hoach hom nay");
    await user.type(screen.getByLabelText("Tìm hội thoại"), "khong co");
    await screen.findByText("Không tìm thấy hội thoại.");
  }, 15000);

  it("gom hoi thoai theo moc thoi gian", async () => {
    const today = new Date();
    const older = new Date(today.getTime() - 20 * 86400000);
    testServer.use(
      http.get("/api/sessions", () =>
        HttpResponse.json({
          sessions: [
            { ...session, id: 10, title: "Ke hoach buoi sang", updated_at: today.toISOString() },
            { ...session, id: 11, title: "Du an cu", updated_at: older.toISOString() },
          ],
        }),
      ),
    );
    renderSidebar();
    await screen.findByText("Ke hoach buoi sang");
    await screen.findByText("Du an cu");
    expect(screen.getByRole("heading", { name: "Hôm nay" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "30 ngày trước" })).toBeInTheDocument();
  }, 15000);

  it("trang chu HUB khong loi danh sach hoi thoai vao", async () => {
    testServer.use(http.get("/api/sessions", () => HttpResponse.json({ sessions: [session] })));
    renderManagement(
      <Routes>
        <Route element={<ChatLayout />}>
          <Route path="/" element={<div>hub</div>} />
        </Route>
      </Routes>,
    );
    await screen.findByText("hub");
    expect(screen.queryByLabelText("Tìm hội thoại")).not.toBeInTheDocument();
  }, 15000);
});
