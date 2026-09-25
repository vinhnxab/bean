import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { delay, HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { MemoryPage } from "@/features/memory/MemoryPage";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

describe("MemoryPage", () => {
  it("hiển thị trạng thái tải", () => {
    testServer.use(
      http.get("/api/memory/files/:name", async () => {
        await delay(100);
        return HttpResponse.json({ name: "MEMORY", content: "" });
      }),
    );
    renderManagement(<MemoryPage />);
    expect(screen.getAllByText("Đang tải…").length).toBeGreaterThan(0);
  });

  it("hiển thị lỗi và yêu cầu xác nhận trước khi lưu", async () => {
    let putBody: unknown = null;
    testServer.use(
      http.get("/api/memory/files/:name", () => HttpResponse.json({ name: "MEMORY", content: "cũ" })),
      http.get("/api/memories", () => HttpResponse.json({ memories: [] })),
      http.put("/api/memory/files/MEMORY", async ({ request }) => {
        putBody = await request.json();
        return HttpResponse.json({ name: "MEMORY", content: "mới" });
      }),
    );
    const user = userEvent.setup();
    renderManagement(<MemoryPage />);
    const editor = await screen.findByLabelText("Ghi chú hệ thống");
    await user.clear(editor);
    await user.type(editor, "mới");
    expect(screen.getAllByText("Thay đổi chưa lưu").length).toBeGreaterThan(0);
    await user.click(screen.getAllByRole("button", { name: "Lưu thay đổi" })[0]);
    expect(screen.getByRole("dialog")).toHaveTextContent("Xác nhận lưu bộ nhớ?");
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Lưu thay đổi" }));
    await waitFor(() => expect(putBody).toEqual({ content: "mới" }));
  });
  it("hiển thị lỗi khi tải file bộ nhớ", async () => {
    testServer.use(
      http.get("/api/memory/files/:name", () =>
        HttpResponse.json({ code: "boom", message: "no" }, { status: 500 }),
      ),
      http.get("/api/memories", () => HttpResponse.json({ memories: [] })),
    );
    renderManagement(<MemoryPage />);
    expect((await screen.findAllByRole("alert")).length).toBeGreaterThan(0);
  });
});
