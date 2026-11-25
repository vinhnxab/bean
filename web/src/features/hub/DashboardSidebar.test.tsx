import { screen, waitFor, within } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { DashboardSidebar } from "@/features/hub/DashboardSidebar";
import { renderManagement } from "@/test/management";
import { MCP_FIXTURE, TOOLS_FIXTURE, testServer, usageFixture } from "@/test/server";

/**
 * Test sidebar tổng quan của Dashboard (đảo kính phải trên trang chủ).
 *
 * Khác với `HubPage` (nội dung chính), sidebar **tự fetch** dữ liệu nên không
 * cần `RealtimeProvider` hay stub WebSocket — chỉ cần MSW trả đủ endpoint.
 * Test mặc định tiếng Anh (`DEFAULT_LANG = "en"`).
 */
function renderSidebar() {
  return renderManagement(<DashboardSidebar open={false} onClose={() => {}} />);
}

describe("DashboardSidebar", () => {
  it("đo token hôm nay theo /api/status, vượt ngân sách thì đỏ", async () => {
    testServer.use(
      http.get("/api/status", () =>
        HttpResponse.json({
          version: "0.1.0",
          model: "fake-model",
          max_steps: 25,
          daily_token_budget: 1_000_000,
          tokens_used: 1_100_000,
          uptime_seconds: 0,
          channels: ["web"],
          restart: { required: false, reasons: [] },
        }),
      ),
    );
    renderSidebar();

    // Panel hiện "—" trước khi dữ liệu về, nên chờ đúng SỐ xuất hiện mới khẳng định.
    await waitFor(() => expect(screen.getByTestId("hub-holo-tokens")).toHaveTextContent("1,100,000"));
    expect(screen.getByTestId("hub-holo-tokens")).toHaveTextContent("1,000,000");
    // Vượt ngân sách: gauge đổi màu cảnh báo VÀ nhãn chữ nói rõ — không chỉ màu.
    expect(screen.getByTestId("hub-holo-tokens-gauge")).toHaveClass("bg-alert");
    expect(screen.getByText("Budget exceeded")).toBeInTheDocument();
  });

  it("vẽ một cột cho mỗi ngày token, tổng khớp dữ liệu", async () => {
    testServer.use(
      http.get("/api/usage", () =>
        HttpResponse.json(
          usageFixture([
            { day: "2026-11-09", total: 1000 },
            { day: "2026-11-10", total: 2000 },
            { day: "2026-11-11", total: 3000 },
          ]),
        ),
      ),
    );
    renderSidebar();

    // Chờ đúng số CỘT xuất hiện — tổng chỉ đọc được sau khi dữ liệu đã về.
    await waitFor(() => expect(screen.getAllByTestId("holo-usage-bar")).toHaveLength(3));
    expect(screen.getByTestId("hub-holo-trend-total")).toHaveTextContent(
      (1000 + 2000 + 3000).toLocaleString(),
    );
  });

  it("bảng hệ thống lấy số thật từ tools, MCP và runtime", async () => {
    // `TOOLS_FIXTURE`: 3 tool hiện ra trong tổng số 5 ⇒ 2 tool bị RBAC ẩn.
    testServer.use(http.get("/api/tools", () => HttpResponse.json(TOOLS_FIXTURE)));
    testServer.use(http.get("/api/mcp", () => HttpResponse.json(MCP_FIXTURE)));
    renderSidebar();

    // "3 / 5" chứ không phải "3": người đọc phải biết registry còn tool bị ẩn.
    await waitFor(() => expect(screen.getByTestId("hub-system-tools")).toHaveTextContent("3 / 5"));
    expect(screen.getByTestId("hub-system-mcp")).toHaveTextContent("1 / 2");
    // Runtime (từ `/api/status` mặc định): bản binary và kênh đang chạy.
    expect(screen.getByTestId("hub-system-version")).toHaveTextContent("0.1.0");
    expect(screen.getByTestId("hub-system-channels")).toHaveTextContent("web");
  });

  it("không bịa số liệu khi endpoint lỗi: hiện gạch ngang, không phải số 0", async () => {
    testServer.use(http.get("/api/status", () => new HttpResponse(null, { status: 500 })));
    testServer.use(http.get("/api/tools", () => new HttpResponse(null, { status: 500 })));
    testServer.use(http.get("/api/mcp", () => new HttpResponse(null, { status: 500 })));
    testServer.use(http.get("/api/usage", () => new HttpResponse(null, { status: 500 })));
    renderSidebar();

    // React Query giữ `data === undefined` khi lỗi; "0" ở đây sẽ khẳng định sai
    // rằng Bean không có tool nào — một lời nói dối lặp thành thói quen.
    await waitFor(() => {
      expect(screen.getByTestId("hub-holo-tokens")).toHaveTextContent("—");
      expect(screen.getByTestId("hub-holo-trend-total")).toHaveTextContent("—");
      expect(screen.getByTestId("hub-system-version")).toHaveTextContent("—");
      expect(screen.getByTestId("hub-system-tools")).toHaveTextContent("—");
      expect(screen.getByTestId("hub-system-mcp")).toHaveTextContent("—");
    });
  });

  it("rút gọn hoạt động, có lối sang màn Audit", async () => {
    // 30 dòng audit: widget chỉ được hiện 5 dòng (đã hỏi server `limit=5`).
    const many = Array.from({ length: 30 }, (_, i) => ({
      ts: new Date(Date.UTC(2026, 10, 11, 10, i)).toISOString(),
      session: "web:admin",
      tool: `tool_${i}`,
      decision: "allow",
      ok: true,
      decided_by: "web:admin",
    }));
    testServer.use(http.get("/api/audit", () => HttpResponse.json({ entries: many })));
    renderSidebar();

    const feed = await screen.findByTestId("hub-activity");
    await waitFor(() => expect(within(feed).getAllByRole("listitem")).toHaveLength(5));
    // Chỉ 5 dòng, và dòng đầu là mới nhất — thứ tự server đã trả.
    expect(within(feed).getByText("tool_0")).toBeInTheDocument();
    expect(within(feed).queryByText("tool_5")).not.toBeInTheDocument();
    // Có lối sang màn Audit đầy đủ, vì widget cố tình không đủ dữ liệu.
    expect(within(feed).getByRole("link", { name: /See all/ })).toHaveAttribute("href", "/audit");
  });

  it("các panel là bảng chỉ đọc, không khai một trạng thái nào có thể ghi", () => {
    renderSidebar();
    // Mỗi panel chỉ hiển thị số liệu: không nút, không ô nhập. Nút đóng ngăn kéo
    // nằm ở header ngoài panel — assert trong phạm vi từng panel chứ không toàn
    // sidebar. Test này để sau này ai thêm control vào panel thì phải dừng lại.
    for (const testId of ["hub-sidebar-tokens", "hub-activity", "hub-system"]) {
      const panel = screen.getByTestId(testId);
      expect(within(panel).queryByRole("button")).toBeNull();
      expect(within(panel).queryByRole("textbox")).toBeNull();
      expect(panel.querySelector("input, select")).toBeNull();
    }
  });
});
