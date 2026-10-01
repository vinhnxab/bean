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

    expect(screen.getByText("Đã kết nối")).toBeInTheDocument();
    expect(screen.getByText("Chưa kết nối")).toBeInTheDocument();
    expect(screen.getByText("4 tool")).toBeInTheDocument();
    expect(screen.getByText("0 tool")).toBeInTheDocument();

    // trust=false → mỗi lời gọi phải xác nhận; trust=true → chạy thẳng (Safe).
    expect(screen.getByText("Mỗi lời gọi phải xác nhận")).toBeInTheDocument();
    expect(screen.getByText("Tool chạy thẳng (Safe)")).toBeInTheDocument();
    expect(screen.getByText("300 giây")).toBeInTheDocument();
    expect(screen.getByText("Mặc định 60 giây")).toBeInTheDocument();
    expect(screen.getByText("infra-read")).toBeInTheDocument();
  });

  it("giải thích 'đã kết nối' không phải health check sống/chết", async () => {
    testServer.use(http.get("/api/mcp", () => HttpResponse.json(MCP_FIXTURE)));
    renderManagement(<McpPage />);
    await screen.findByText("siem");
    expect(screen.getByText(/không phải kiểm tra sống\/chết/)).toBeInTheDocument();
  });

  it("không có server nào thì hướng dẫn cách khai báo", async () => {
    renderManagement(<McpPage />);
    expect(await screen.findByText("Chưa có MCP server nào")).toBeInTheDocument();
    expect(screen.getByText(/bean\.toml/)).toBeInTheDocument();
    expect(screen.queryByText(/Đã kết nối/)).not.toBeInTheDocument();
  });

  it("lỗi API hiển thị dạng alert, không im lặng", async () => {
    testServer.use(http.get("/api/mcp", () => HttpResponse.json({ code: "boom" }, { status: 500 })));
    renderManagement(<McpPage />);
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });
});
