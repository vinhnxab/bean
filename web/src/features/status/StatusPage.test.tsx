import { screen } from "@testing-library/react";
import { delay, HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { StatusPage } from "@/features/status/StatusPage";
import { renderManagement } from "@/test/management";
import { testServer, usageFixture } from "@/test/server";

describe("StatusPage", () => {
  it("hiển thị trạng thái tải", () => {
    testServer.use(
      http.get("/api/status", async () => {
        await delay(100);
        return HttpResponse.json({});
      }),
    );
    renderManagement(<StatusPage />);
    expect(screen.getByRole("status")).toHaveTextContent("Đang tải");
  });

  it("hiển thị version, model, ngân sách và channel", async () => {
    testServer.use(
      http.get("/api/status", () =>
        HttpResponse.json({
          version: "0.1.0",
          model: "test-model",
          max_steps: 25,
          daily_token_budget: 2000,
          tokens_used: 500,
          uptime_seconds: 3660,
          channels: ["web", "telegram"],
        }),
      ),
    );
    renderManagement(<StatusPage />);
    expect(await screen.findByText("0.1.0")).toBeInTheDocument();
    expect(screen.getByText("test-model")).toBeInTheDocument();
    expect(screen.getByText("25")).toBeInTheDocument();
    expect(screen.getByText("500 / 2,000")).toBeInTheDocument();
    expect(screen.getByText("telegram")).toBeInTheDocument();
  });

  it("hiển thị lỗi", async () => {
    testServer.use(
      http.get("/api/status", () => HttpResponse.json({ code: "boom", message: "no" }, { status: 500 })),
    );
    renderManagement(<StatusPage />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Đã xảy ra lỗi.");
  });

  it("hiển thị xu hướng token 14 ngày cạnh số của hôm nay", async () => {
    testServer.use(
      http.get("/api/usage", () =>
        HttpResponse.json(
          usageFixture([
            { day: "2026-09-28", total: 900 },
            { day: "2026-09-29", total: 400 },
          ]),
        ),
      ),
    );
    renderManagement(<StatusPage />);
    expect(await screen.findAllByTestId("usage-bar")).toHaveLength(2);
    expect(screen.getByText("Token 14 ngày")).toBeInTheDocument();
    expect(screen.getByText("1,300")).toBeInTheDocument();
  });

  it("panel cấu hình cho thấy sandbox và bind address thật", async () => {
    renderManagement(<StatusPage />);
    expect(await screen.findByText("bean-sandbox:latest")).toBeInTheDocument();
    expect(screen.getByText("docker")).toBeInTheDocument();
    expect(screen.getByText("127.0.0.1:7878")).toBeInTheDocument();
    expect(screen.getByText(/RBAC đang tắt/)).toBeInTheDocument();
  });

  it("panel cấu hình chỉ in **tên** biến API key, không có chỗ cho giá trị", async () => {
    renderManagement(<StatusPage />);
    expect(await screen.findByText("ANTHROPIC_API_KEY")).toBeInTheDocument();
    // `/api/system` đã khử secret phía server; UI cũng không được tự thêm gì có dạng key.
    expect(document.body.textContent).not.toMatch(/sk-[A-Za-z0-9-]{8,}/);
    expect(document.body.textContent).not.toMatch(/token\s*[:=]\s*[A-Za-z0-9_-]{16,}/i);
  });
});
