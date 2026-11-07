import { screen } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { McpPage } from "@/features/mcp/McpPage";
import { renderManagement } from "@/test/management";
import { MCP_FIXTURE, testServer } from "@/test/server";

describe("McpPage", () => {
  it("hiển thị lệnh đầy đủ, số tool đã discovery, trạng thái và mức tin cậy", async () => {
    testServer.use(http.get("/api/mcp", () => HttpResponse.json(MCP_FIXTURE)));
    renderManagement(<McpPage />);

    expect(await screen.findByText("siem")).toBeInTheDocument();
    expect(screen.getByText("weather")).toBeInTheDocument();
    // Nguyên văn lệnh khởi động: người dùng phải thấy server chạy bằng gì.
    expect(screen.getByText("docker run --rm siem-mcp")).toBeInTheDocument();
    expect(screen.getByText("/usr/local/bin/weather-mcp")).toBeInTheDocument();

    expect(screen.getByText("Connected")).toBeInTheDocument();
    expect(screen.getByText("Not connected")).toBeInTheDocument();
    expect(screen.getByText("4 tools")).toBeInTheDocument();
    expect(screen.getByText("0 tools")).toBeInTheDocument();

    // trust=false → mỗi lời gọi phải xác nhận; trust=true → chạy thẳng (Safe).
    expect(screen.getByText("Every call needs confirmation")).toBeInTheDocument();
    expect(screen.getByText("Tools run directly (Safe)")).toBeInTheDocument();
    expect(screen.getByText("300 seconds")).toBeInTheDocument();
    expect(screen.getByText("Default 60 seconds")).toBeInTheDocument();
    expect(screen.getByText("infra-read")).toBeInTheDocument();
  });

  it("giải thích 'đã kết nối' không phải health check sống/chết", async () => {
    testServer.use(http.get("/api/mcp", () => HttpResponse.json(MCP_FIXTURE)));
    renderManagement(<McpPage />);
    await screen.findByText("siem");
    expect(screen.getByText(/not a liveness check/)).toBeInTheDocument();
  });

  it("không có server nào thì hướng dẫn cách khai báo", async () => {
    renderManagement(<McpPage />);
    expect(await screen.findByText("No MCP servers")).toBeInTheDocument();
    expect(screen.getByText(/bean\.toml/)).toBeInTheDocument();
    expect(screen.queryByText(/Connected/)).not.toBeInTheDocument();
  });

  it("lỗi API hiển thị dạng alert, không im lặng", async () => {
    testServer.use(http.get("/api/mcp", () => HttpResponse.json({ code: "boom" }, { status: 500 })));
    renderManagement(<McpPage />);
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });
});
