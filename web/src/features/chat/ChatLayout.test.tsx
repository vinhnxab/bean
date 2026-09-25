import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";

import { ChatLayout } from "@/features/chat/ChatLayout";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const session = {
  id: 4,
  channel: "web",
  chat_id: "web:admin",
  user_id: "web:admin",
  title: "Kế hoạch hôm nay",
  archived: false,
  created_at: "2026-09-26T00:00:00Z",
  updated_at: "2026-09-26T00:00:00Z",
};

describe("ChatLayout sidebar", () => {
  it("tìm, đổi tên, lưu trữ và xoá hội thoại", async () => {
    let patchBody: unknown = null;
    let deleteCalled = false;
    testServer.use(
      http.get("/api/sessions", ({ request }) => {
        const query = new URL(request.url).searchParams.get("q");
        return HttpResponse.json({ sessions: query ? [] : [session] });
      }),
      http.patch("/api/sessions/4", async ({ request }) => {
        patchBody = await request.json();
        return HttpResponse.json({ ok: true });
      }),
      http.delete("/api/sessions/4", () => {
        deleteCalled = true;
        return HttpResponse.json({ deleted: true });
      }),
    );
    const user = userEvent.setup();
    renderManagement(
      <Routes>
        <Route element={<ChatLayout />}>
          <Route path="/" element={<div>chat</div>} />
        </Route>
      </Routes>,
    );
    expect(await screen.findByText("Kế hoạch hôm nay")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Tìm hội thoại"), "không có");
    expect(await screen.findByText("Không tìm thấy hội thoại.")).toBeInTheDocument();
    await user.clear(screen.getByLabelText("Tìm hội thoại"));
    await user.click(screen.getByRole("button", { name: "Đổi tên Kế hoạch hôm nay" }));
    const input = screen.getByLabelText("Đổi tên");
    await user.clear(input);
    await user.type(input, "Tên mới");
    await user.click(screen.getByRole("button", { name: "Lưu" }));
    await waitFor(() => expect(patchBody).toEqual({ title: "Tên mới", archived: null }));
    await user.click(screen.getByRole("button", { name: "Lưu trữ" }));
    await waitFor(() => expect(patchBody).toEqual({ title: null, archived: true }));
    await user.click(screen.getByRole("button", { name: "Xoá Kế hoạch hôm nay" }));
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Xoá" }));
    await waitFor(() => expect(deleteCalled).toBe(true));
  }, 15000);
});
