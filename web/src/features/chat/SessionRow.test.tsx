import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import type { SessionDto } from "@/api/bindings";
import { SessionRow } from "@/features/chat/ChatLayout";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const session: SessionDto = {
  id: 4,
  channel: "web",
  chat_id: "web:admin",
  user_id: "web:admin",
  title: "Ke hoach",
  archived: false,
  created_at: "2026-09-26T00:00:00Z",
  updated_at: "2026-09-26T00:00:00Z",
};

function renderRow(overrides: Partial<SessionDto> = {}) {
  const user = userEvent.setup();
  renderManagement(
    <ul>
      <SessionRow session={{ ...session, ...overrides }} onDeleted={() => {}} onNavigate={() => {}} />
    </ul>,
  );
  return user;
}

/**
 * Mở menu dòng hội thoại bằng **bàn phím** rồi bấm một mục.
 *
 * Vì sao không dùng `user.click` vào nút menu: Radix `DropdownMenu` giữ
 * `DismissableLayer` ở phạm vi module, và các lớp của lần render trước chưa
 * gỡ hết khiến cú click chuỗi-pointer ở test sau bị bỏ qua — biểu hiện là
 * `data-state` vẫn `closed` và test fail với thông báo rất sai lệch ("không
 * tìm thấy menuitem"). Triệu chứng đặc trưng: test đầu tiên xanh, các test
 * sau đều đỏ.
 *
 * Bàn phím đi đường khác (`Enter` trên trigger đang focus) nên không dính
 * trạng thái pointer còn sót, và đúng bằng cách người dùng thật mở menu.
 */
async function pickAction(user: ReturnType<typeof userEvent.setup>, name: string) {
  const trigger = screen.getByRole("button", { name: "Thao tác khác" });
  trigger.focus();
  await user.keyboard("{Enter}");
  const item = await screen.findByRole("menuitem", { name });
  item.focus();
  await user.keyboard("{Enter}");
}

describe("SessionRow menu", () => {
  it("rename", async () => {
    let body: unknown = null;
    testServer.use(
      http.patch("/api/sessions/4", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({ ok: true });
      }),
    );
    const user = renderRow();
    await pickAction(user, "Đổi tên");
    const input = screen.getByLabelText("Đổi tên");
    await user.clear(input);
    await user.type(input, "Ten moi");
    await user.click(screen.getByRole("button", { name: "Lưu" }));
    await expect.poll(() => body, { timeout: 5000 }).toEqual({ title: "Ten moi", archived: null });
  });

  it("archive", async () => {
    let body: unknown = null;
    testServer.use(
      http.patch("/api/sessions/4", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({ ok: true });
      }),
    );
    const user = renderRow();
    await pickAction(user, "Lưu trữ");
    await expect.poll(() => body, { timeout: 5000 }).toEqual({ title: null, archived: true });
  });

  it("unarchive", async () => {
    let body: unknown = null;
    testServer.use(
      http.patch("/api/sessions/4", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({ ok: true });
      }),
    );
    const user = renderRow({ archived: true });
    await pickAction(user, "Bỏ lưu trữ");
    await expect.poll(() => body, { timeout: 5000 }).toEqual({ title: null, archived: false });
  });

  it("delete needs confirm", async () => {
    let called = false;
    testServer.use(
      http.delete("/api/sessions/4", () => {
        called = true;
        return HttpResponse.json({ deleted: true });
      }),
    );
    const user = renderRow();
    await pickAction(user, "Xoá");
    await screen.findByRole("dialog");
    expect(called).toBe(false);
    await user.click(screen.getByRole("button", { name: "Xoá" }));
    await expect.poll(() => called, { timeout: 5000 }).toBe(true);
  });
});
