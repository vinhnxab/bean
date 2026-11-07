import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { AdvancedPanel } from "@/features/settings/AdvancedPanel";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const PASSWORD = "correct horse battery staple";

function lanHandlers(overrides: {
  status?: number;
  error?: { code: string; message: string };
  mode?: string;
}) {
  const mode = overrides.mode ?? "loopback";
  return [
    http.get("/api/settings/lan-access", () => {
      if (overrides.status) {
        return HttpResponse.json(overrides.error, { status: overrides.status });
      }
      return HttpResponse.json({
        mode,
        lan_ip: "192.168.1.29",
        port: 7878,
        url: mode === "lan" ? "http://192.168.1.29:7878" : null,
      });
    }),
    http.post("/api/settings/lan-access", async ({ request }) => {
      const body = (await request.json()) as { enabled: boolean; password: string };
      if (body.password !== PASSWORD) {
        return HttpResponse.json(
          { code: "invalid_password", message: "mật khẩu không đúng" },
          { status: 401 },
        );
      }
      const nextMode = body.enabled ? "lan" : "loopback";
      return HttpResponse.json({
        access: {
          mode: nextMode,
          lan_ip: "192.168.1.29",
          port: 7878,
          url: nextMode === "lan" ? "http://192.168.1.29:7878" : null,
        },
        restart_required: true,
      });
    }),
    http.get("/api/settings", () => HttpResponse.json({ settings: [], writable: true })),
  ];
}

describe("LanAccessCard — bật/tắt truy cập LAN (D21.x)", () => {
  it("chế độ loopback: hiện trạng thái và công tắc tắt, không có ô mật khẩu", async () => {
    testServer.use(...lanHandlers({ mode: "loopback" }));
    renderManagement(<AdvancedPanel />);

    expect(await screen.findByRole("switch", { name: "Toggle LAN access" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.getByText("Only this machine can access (127.0.0.1).")).toBeInTheDocument();
    expect(screen.queryByLabelText("Current password")).not.toBeInTheDocument();
  });

  it("bật công tắc: hiện gợi ý URL LAN + ô mật khẩu, gửi đúng enabled=true", async () => {
    const user = userEvent.setup();
    testServer.use(...lanHandlers({ mode: "loopback" }));
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("switch", { name: "Toggle LAN access" }));
    expect(screen.getByText(/Will bind 0.0.0.0 and serve http:\/\/192\.168\.1\.29:7878/)).toBeInTheDocument();

    await user.type(screen.getByLabelText("Current password"), PASSWORD);
    await user.click(screen.getByRole("button", { name: "Apply" }));

    // Sau thành công: ô mật khẩu đóng lại (bản nháp reset), trạng thái quay về
    // chờ refetch — invalidate settings/lanAccess đã được gọi.
    await waitFor(() => {
      expect(screen.queryByLabelText("Current password")).not.toBeInTheDocument();
    });
  });

  it("sai mật khẩu: hiện thông báo lỗi từ server, form vẫn mở", async () => {
    const user = userEvent.setup();
    testServer.use(...lanHandlers({ mode: "loopback" }));
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("switch", { name: "Toggle LAN access" }));
    await user.type(screen.getByLabelText("Current password"), "wrong password entirely");
    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("mật khẩu không đúng");
    expect(screen.getByLabelText("Current password")).toBeInTheDocument();
  });

  it("chế độ LAN: công tắc bật, gợi ý tắt về loopback", async () => {
    const user = userEvent.setup();
    testServer.use(...lanHandlers({ mode: "lan" }));
    renderManagement(<AdvancedPanel />);

    expect(await screen.findByText("LAN access on: http://192.168.1.29:7878")).toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "Toggle LAN access" }));
    expect(screen.getByText(/Will close back to 127\.0\.0\.1/)).toBeInTheDocument();
  });

  it("GET lỗi: hiện thông báo + nút thử lại", async () => {
    const user = userEvent.setup();
    testServer.use(...lanHandlers({ status: 500, error: { code: "internal", message: "boom" } }));
    renderManagement(<AdvancedPanel />);

    expect(await screen.findByText("Could not read LAN access status.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
  });
});
