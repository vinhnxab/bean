import { screen } from "@testing-library/react";
import { delay, HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { StatusPage } from "@/features/status/StatusPage";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

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
});
