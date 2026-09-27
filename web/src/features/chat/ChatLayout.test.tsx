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
    // Render ở route chat, không phải `/`: `/` giờ là HUB và cố tình **không** hiện
    // danh sách hội thoại. Test sidebar phải đứng ở nơi sidebar thực sự xuất hiện.
    renderManagement(
      <Routes>
        <Route element={<ChatLayout />}>
          <Route path="/" element={<div>hub</div>} />
          <Route path="/sessions/:sessionId" element={<div>chat</div>} />
        </Route>
      </Routes>,
      ["/sessions/4"],
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

  it("trang chủ HUB không lôi danh sách hội thoại vào", async () => {
    testServer.use(http.get("/api/sessions", () => HttpResponse.json({ sessions: [session] })));
    renderManagement(
      <Routes>
        <Route element={<ChatLayout />}>
          <Route path="/" element={<div>hub</div>} />
        </Route>
      </Routes>,
    );
    // Sidebar hội thoại là ngữ cảnh của trang chat; đặt cạnh sơ đồ hệ agent chỉ
    // chiếm nửa màn hình và làm loãng thứ người dùng cần thấy đầu tiên.
    await waitFor(() => expect(screen.getByText("hub")).toBeInTheDocument());
    expect(screen.queryByLabelText("Tìm hội thoại")).not.toBeInTheDocument();
  });
});
