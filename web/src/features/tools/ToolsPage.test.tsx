import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { ToolsPage } from "@/features/tools/ToolsPage";
import { renderManagement } from "@/test/management";
import { TOOLS_FIXTURE, testServer } from "@/test/server";

const SEARCH_PLACEHOLDER = "Tìm theo tên, mô tả hoặc tag";

describe("ToolsPage", () => {
  it("hiển thị tool, mô tả model đang đọc, tag và nguồn MCP", async () => {
    testServer.use(http.get("/api/tools", () => HttpResponse.json(TOOLS_FIXTURE)));
    renderManagement(<ToolsPage />);

    expect(await screen.findByText("read_file")).toBeInTheDocument();
    expect(screen.getByText("run_shell")).toBeInTheDocument();
    expect(screen.getByText("Đọc một file trong workspace.")).toBeInTheDocument();
    expect(screen.getByText("MCP · siem")).toBeInTheDocument();
    expect(screen.getByText("dev-write")).toBeInTheDocument();
    // Mọi tool đọc nội dung ngoài đều phải gắn nhãn "không tin cậy": UI phải escape
    // và mọi tool Confirm phải hỏi lại sau khi thấy nó (mục 15.4 của spec).
    expect(screen.getAllByText("Nội dung không tin cậy")).toHaveLength(3);
    expect(screen.getAllByText("Cần xác nhận")).toHaveLength(3);
  });

  it("nói thẳng RBAC đang cắt danh sách, không để người dùng tưởng registry có ít tool", async () => {
    testServer.use(http.get("/api/tools", () => HttpResponse.json(TOOLS_FIXTURE)));
    renderManagement(<ToolsPage />);
    await screen.findByText("read_file");
    expect(screen.getByText(/finance-readonly được phép thấy 3\/5/)).toBeInTheDocument();
    expect(screen.getByText("Tool bạn thấy")).toBeInTheDocument();
  });

  it("lọc chỉ là tiện ích: không gọi lại API và không lộ tool bị RBAC ẩn", async () => {
    let calls = 0;
    testServer.use(
      http.get("/api/tools", () => {
        calls += 1;
        return HttpResponse.json(TOOLS_FIXTURE);
      }),
    );
    const user = userEvent.setup();
    renderManagement(<ToolsPage />);
    await screen.findByText("read_file");
    expect(calls).toBe(1);

    const input = screen.getByPlaceholderText(SEARCH_PLACEHOLDER);
    await user.type(input, "siem");
    expect(screen.queryByText("read_file")).not.toBeInTheDocument();
    expect(screen.getByText("mcp__siem__cve_lookup")).toBeInTheDocument();

    await user.clear(input);
    await user.type(input, "dev-write"); // tìm theo tag quyền
    expect(screen.getByText("run_shell")).toBeInTheDocument();
    expect(screen.queryByText("mcp__siem__cve_lookup")).not.toBeInTheDocument();
    expect(calls).toBe(1);
  }, 15000);

  it("lọc không ra kết quả thì nói khác với registry trống", async () => {
    testServer.use(http.get("/api/tools", () => HttpResponse.json(TOOLS_FIXTURE)));
    const user = userEvent.setup();
    renderManagement(<ToolsPage />);
    await screen.findByText("read_file");
    await user.type(screen.getByPlaceholderText(SEARCH_PLACEHOLDER), "khong-ton-tai");
    expect(await screen.findByText("Không có kết quả")).toBeInTheDocument();
    expect(screen.queryByText(/Registry trống/)).not.toBeInTheDocument();
  }, 15000);

  it("registry trống thì chỉ thẳng chỗ cấu hình", async () => {
    renderManagement(<ToolsPage />);
    expect(await screen.findByText("Không có tool nào")).toBeInTheDocument();
    expect(screen.getByText(/Registry trống/)).toBeInTheDocument();
  });
});
