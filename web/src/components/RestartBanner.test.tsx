import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { beforeEach, describe, expect, it } from "vitest";

import { RestartBanner } from "@/components/RestartBanner";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const PENDING_STATUS = {
  version: "0.1.0",
  model: "test-model",
  max_steps: 25,
  daily_token_budget: 2000,
  tokens_used: 0,
  uptime_seconds: 60,
  channels: ["web"],
  restart: {
    required: true,
    reasons: ["1 khoá cấu hình chưa hiệu lực: web.bind", "bản mới v1.2.3 đã cài"],
  },
};

describe("RestartBanner", () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  it("ẩn khi không có gì chờ restart", () => {
    renderManagement(<RestartBanner />);
    // `/api/status` mặc định (fixture server.ts) có restart.required = false.
    expect(screen.queryByTestId("restart-banner")).not.toBeInTheDocument();
  });

  it("backend cũ thiếu field restart: ẩn banner, không crash (UI mới hơn binary)", () => {
    // ui_dir phục vụ UI từ đĩa — UI có thể chạy trước binary chưa có M31.
    // Lỗi thật 2026-11-24: `reading 'required' of undefined` crash cả app.
    testServer.use(
      http.get("/api/status", () =>
        HttpResponse.json({
          version: "0.1.0",
          model: "test-model",
          max_steps: 25,
          daily_token_budget: 2000,
          tokens_used: 0,
          uptime_seconds: 60,
          channels: ["web"],
          // cố ý KHÔNG có `restart`
        }),
      ),
    );
    renderManagement(<RestartBanner />);
    expect(screen.queryByTestId("restart-banner")).not.toBeInTheDocument();
  });

  it("hiện banner kèm lý do khi restart.required", async () => {
    testServer.use(http.get("/api/status", () => HttpResponse.json(PENDING_STATUS)));
    renderManagement(<RestartBanner />);
    expect(await screen.findByTestId("restart-banner")).toBeInTheDocument();
    expect(screen.getByText("Bean needs a restart")).toBeInTheDocument();
    expect(screen.getByText("bản mới v1.2.3 đã cài")).toBeInTheDocument();
  });

  it("'Later' ẩn banner trong phiên (sessionStorage) và không gọi POST", async () => {
    const user = userEvent.setup();
    let restartCalled = 0;
    testServer.use(
      http.get("/api/status", () => HttpResponse.json(PENDING_STATUS)),
      http.post("/api/system/restart", () => {
        restartCalled += 1;
        return HttpResponse.json({ restarting: true }, { status: 202 });
      }),
    );
    renderManagement(<RestartBanner />);
    await user.click(await screen.findByRole("button", { name: "Later" }));
    await waitFor(() => {
      expect(screen.queryByTestId("restart-banner")).not.toBeInTheDocument();
    });
    expect(sessionStorage.getItem("bean.restart-banner-dismissed")).toBe("1");
    expect(restartCalled).toBe(0);
  });

  it("'Restart now' → 202 → chuyển sang đang khởi động lại, khoá nút", async () => {
    const user = userEvent.setup();
    testServer.use(
      http.get("/api/status", () => HttpResponse.json(PENDING_STATUS)),
      http.post("/api/system/restart", () => HttpResponse.json({ restarting: true }, { status: 202 })),
    );
    renderManagement(<RestartBanner />);
    await user.click(await screen.findByRole("button", { name: "Restart now" }));
    expect(await screen.findByText("Restarting…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart now" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Later" })).toBeDisabled();
  });

  it("409 run_active: hiện cảnh báo, banner giữ nguyên", async () => {
    const user = userEvent.setup();
    testServer.use(
      http.get("/api/status", () => HttpResponse.json(PENDING_STATUS)),
      http.post("/api/system/restart", () =>
        HttpResponse.json({ code: "run_active", message: "đang có 1 run chạy" }, { status: 409 }),
      ),
    );
    renderManagement(<RestartBanner />);
    await user.click(await screen.findByRole("button", { name: "Restart now" }));
    expect(
      await screen.findByText(/A run is active — stop it \(or \/stop\) first, then retry\./),
    ).toBeInTheDocument();
    // Nút không bị khoá: người dùng dừng run rồi bấm lại được.
    expect(screen.getByRole("button", { name: "Restart now" })).toBeEnabled();
  });
});
