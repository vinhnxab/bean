import { screen, waitFor } from "@testing-library/react";
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
    expect(await screen.findByRole("alert")).toHaveTextContent("Đã xảy ra lỗi.");
  });

  it("tạo và bật/tắt tác vụ", async () => {
    let created: unknown = null;
    let toggled: unknown = null;
    testServer.use(
      http.get("/api/tasks", () => HttpResponse.json({ tasks: [task] })),
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
    await user.click(screen.getByRole("button", { name: "Tắt" }));
    await waitFor(() => expect(toggled).toEqual({ enabled: false }));
    await user.type(screen.getByLabelText("Prompt"), "Tạo task mới");
    await user.click(screen.getByRole("button", { name: "Tạo tác vụ" }));
    await waitFor(() => expect(created).toMatchObject({ prompt: "Tạo task mới", cron: "0 9 * * *" }));
  });
  it("hiển thị trạng thái tải", () => {
    testServer.use(
      http.get("/api/tasks", async () => {
        await delay(100);
        return HttpResponse.json({ tasks: [] });
      }),
    );
    renderManagement(<TasksPage />);
    expect(screen.getByText("Đang tải…")).toBeInTheDocument();
  });
});
