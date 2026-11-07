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
    expect(screen.getAllByText("Loading…").length).toBeGreaterThan(0);
  });

  it("hiển thị lỗi và yêu cầu xác nhận trước khi lưu", async () => {
    let putCalled = false;
    testServer.use(
      http.get("/api/memory/files/:name", () => HttpResponse.json({ name: "MEMORY", content: "cũ" })),
      http.get("/api/memories", () => HttpResponse.json({ memories: [] })),
      http.put("/api/memory/files/MEMORY", async () => {
        putCalled = true;
        return HttpResponse.json({ name: "MEMORY", content: "mới" });
      }),
    );
    const user = userEvent.setup();
    renderManagement(<MemoryPage />);
    const editor = await screen.findByLabelText("System notes");
    await user.clear(editor);
    await user.paste("mới");
    expect(screen.getAllByText("Unsaved changes").length).toBeGreaterThan(0);
    await user.click(screen.getAllByRole("button", { name: "Save changes" })[0]);
    expect(screen.getByRole("dialog")).toHaveTextContent("Confirm memory save?");
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(putCalled).toBe(true));
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
