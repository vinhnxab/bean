import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { delay, HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { TasksPage } from "@/features/tasks/TasksPage";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const task = {
  id: 7,
  cron: "0 9 * * *",
  prompt: "Tóm tắt hôm nay",
  channel: "web",
  chat_id: "web:admin",
  allowed_tools: [],
  next_run: "2026-09-26T09:00:00Z",
  enabled: true,
};

describe("TasksPage", () => {
  it("hiển thị lỗi tải danh sách", async () => {
    testServer.use(
      http.get("/api/tasks", () => HttpResponse.json({ code: "boom", message: "no" }, { status: 500 })),
    );
    renderManagement(<TasksPage />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Something went wrong.");
  });

  it("tạo và bật/tắt tác vụ", async () => {
    let created: unknown = null;
    let toggled: unknown = null;
    let listFetches = 0;
    testServer.use(
      http.get("/api/tasks", () => {
        listFetches += 1;
        // Lần đầu tải trang: task đang bật; refetch sau PATCH: server đã áp dụng.
        const enabled = listFetches === 1;
        return HttpResponse.json({ tasks: [{ ...task, enabled }] });
      }),
      http.post("/api/tasks", async ({ request }) => {
        created = await request.json();
        return HttpResponse.json(task, { status: 201 });
      }),
      http.patch("/api/tasks/7", async ({ request }) => {
        toggled = await request.json();
        return HttpResponse.json({ ...task, enabled: false });
      }),
    );
    const user = userEvent.setup();
    renderManagement(<TasksPage />);
    expect(await screen.findByText("Tóm tắt hôm nay")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Disable" }));
    // Đợi tín hiệu UI của pipeline PATCH (refetch sau invalidate đã render), không
    // đợi biến set trong handler msw — biến được gán trước khi mutation xong, làm
    // bước gõ tiếp theo chồng lên refetch và làm rơi ký tự trên máy CI chậm.
    await screen.findByText("Disabled");
    expect(toggled).toEqual({ enabled: false });
    // Nhập bằng fireEvent.change thay vì gõ từng ký tự: userEvent.type từng làm
    // rơi 2 ký tự cuối trên controlled textarea khi worker khác tải cao (flake
    // CI lặp 3 lần, tái hiện full-suite local) — mỗi ký tự là một event riêng có
    // cửa sổ race với flush của React; fireEvent bọc act nên commit đồng bộ.
    fireEvent.change(screen.getByLabelText("Prompt"), { target: { value: "Tạo task mới" } });
    await user.click(screen.getByRole("button", { name: "Create task" }));
    // Create thành công thì form tự xoá prompt — chỉ assert body sau tín hiệu này.
    await waitFor(() => expect(screen.getByLabelText("Prompt")).toHaveValue(""));
    expect(created).toMatchObject({ prompt: "Tạo task mới", cron: "0 9 * * *" });
  });
  it("hiển thị trạng thái tải", () => {
    testServer.use(
      http.get("/api/tasks", async () => {
        await delay(100);
        return HttpResponse.json({ tasks: [] });
      }),
    );
    renderManagement(<TasksPage />);
    expect(screen.getByText("Loading…")).toBeInTheDocument();
  });
});
