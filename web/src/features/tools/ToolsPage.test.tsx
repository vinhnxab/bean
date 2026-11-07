import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { ToolsPage } from "@/features/tools/ToolsPage";
import { renderManagement } from "@/test/management";
import { TOOLS_FIXTURE, testServer } from "@/test/server";

const SEARCH_PLACEHOLDER = "Search by name, description or tag";

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
    expect(screen.getAllByText("Untrusted content")).toHaveLength(3);
    expect(screen.getAllByText("Confirmation required")).toHaveLength(3);
  });

  it("nói thẳng RBAC đang cắt danh sách, không để người dùng tưởng registry có ít tool", async () => {
    testServer.use(http.get("/api/tools", () => HttpResponse.json(TOOLS_FIXTURE)));
    renderManagement(<ToolsPage />);
    await screen.findByText("read_file");
    expect(screen.getByText(/finance-readonly may see 3 of 5/)).toBeInTheDocument();
    expect(screen.getByText("Tools you can see")).toBeInTheDocument();
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
    expect(await screen.findByText("No matches")).toBeInTheDocument();
    expect(screen.queryByText(/registry is empty/)).not.toBeInTheDocument();
  }, 15000);

  it("registry trống thì chỉ thẳng chỗ cấu hình", async () => {
    renderManagement(<ToolsPage />);
    expect(await screen.findByText("No tools")).toBeInTheDocument();
    expect(screen.getByText(/registry is empty/)).toBeInTheDocument();
  });

  it("hiển thị đúng trạng thái bật/tắt từng tool", async () => {
    testServer.use(http.get("/api/tools", () => HttpResponse.json(TOOLS_FIXTURE)));
    renderManagement(<ToolsPage />);
    await screen.findByText("read_file");
    expect(screen.getAllByText("Enabled")).toHaveLength(2);
    expect(screen.getByText("Disabled")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Enable/disable tool run_shell" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  it("role không phải admin thấy switch khoá và không gọi được API", async () => {
    let toggles = 0;
    testServer.use(
      http.get("/api/tools", () => HttpResponse.json(TOOLS_FIXTURE)),
      http.put("/api/tools/:name/enabled", () => {
        toggles += 1;
        return HttpResponse.json({ name: "run_shell", enabled: true });
      }),
    );
    const user = userEvent.setup();
    renderManagement(<ToolsPage />);
    await screen.findByText("read_file");

    const switchInput = screen.getByRole("switch", { name: "Enable/disable tool run_shell" });
    expect(switchInput).toBeDisabled();
    await user.click(switchInput);
    expect(toggles).toBe(0);
  });

  it("admin gạt switch thì gọi PUT và nạp lại danh sách", async () => {
    let toggles = 0;
    let toolLists = 0;
    testServer.use(
      http.get("/api/tools", () => {
        toolLists += 1;
        return HttpResponse.json({
          ...TOOLS_FIXTURE,
          viewer_role: "admin",
          rbac_enabled: false,
        });
      }),
      http.put("/api/tools/:name/enabled", async ({ request }) => {
        toggles += 1;
        const url = new URL(request.url);
        const name = url.pathname.split("/")[3];
        const body = (await request.json()) as { enabled: boolean };
        return HttpResponse.json({ name, enabled: body.enabled });
      }),
    );
    const user = userEvent.setup();
    renderManagement(<ToolsPage />);
    await screen.findByText("read_file");
    expect(toolLists).toBe(1);

    await user.click(screen.getByRole("switch", { name: "Enable/disable tool run_shell" }));
    expect(toggles).toBe(1);
    // Mutation thành công → invalidate → nạp lại danh sách (GET thứ hai).
    expect(await screen.findByText("Disabled")).toBeInTheDocument();
    expect(toolLists).toBe(2);
  });
});
