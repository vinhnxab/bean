import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import { AdvancedPanel } from "@/features/settings/AdvancedPanel";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const OS_PASSWORD = "correct horse battery staple";

type HostModePost = { enabled: boolean; os_password: string | null };

/**
 * Panel Advanced mount cả card LAN — hai handler này giữ card hàng xóm im lặng
 * (không phát `alert` riêng) để `findByRole("alert")` chỉ trúng lỗi host mode.
 */
const quietNeighbourHandlers = [
  http.get("/api/settings/lan-access", () =>
    HttpResponse.json({ mode: "loopback", lan_ip: "192.168.1.29", port: 7878, url: null }),
  ),
  http.get("/api/settings", () => HttpResponse.json({ settings: [], writable: true })),
];

/**
 * Handler host-mode theo kịch bản, mirror `lanHandlers` của LanAccessCard.test.
 * GET có trạng thái (`diskMode`) để test "đúng mật khẩu" thấy được kết quả ghi
 * đĩa sau khi card invalidate query: lần GET sau trả mode mới + `pending = true`
 * ⇒ gợi ý restart hiện ra.
 */
function hostModeHandlers(overrides: {
  status?: number;
  error?: { code: string; message: string };
  /** `false` = máy không đặt mật khẩu tài khoản OS (bỏ qua step-up). */
  osPasswordRequired?: boolean;
  correctPassword?: string;
}) {
  const osPasswordRequired = overrides.osPasswordRequired ?? true;
  const state: { diskMode: string; posted: HostModePost | null } = { diskMode: "docker", posted: null };
  const handlers = [
    http.get("/api/settings/host-mode", () => {
      if (overrides.status) {
        return HttpResponse.json(overrides.error, { status: overrides.status });
      }
      return HttpResponse.json({
        mode: state.diskMode,
        running_mode: "docker",
        os_password_required: osPasswordRequired,
        pending: state.diskMode !== "docker",
      });
    }),
    http.post("/api/settings/host-mode", async ({ request }) => {
      const body = (await request.json()) as HostModePost;
      state.posted = body;
      // Mirror backend: no-op (mode đĩa đã trùng) không cần step-up.
      const desired = body.enabled ? "host" : "docker";
      if (desired === state.diskMode) {
        return HttpResponse.json({ mode: state.diskMode, restart_required: false });
      }
      if (osPasswordRequired) {
        if (body.os_password === null) {
          return HttpResponse.json(
            { code: "os_password_required", message: "cần mật khẩu máy để bật/tắt host mode" },
            { status: 401 },
          );
        }
        if (body.os_password !== overrides.correctPassword) {
          return HttpResponse.json(
            { code: "invalid_os_password", message: "mật khẩu máy không đúng" },
            { status: 401 },
          );
        }
      }
      state.diskMode = desired;
      return HttpResponse.json({ mode: desired, restart_required: true });
    }),
  ];
  return { handlers, state };
}

describe("HostModeCard — bật/tắt host mode (D22.2)", () => {
  it("máy có mật khẩu: switch tắt ở mode docker, chưa có ô mật khẩu", async () => {
    testServer.use(...quietNeighbourHandlers, ...hostModeHandlers({}).handlers);
    renderManagement(<AdvancedPanel />);

    expect(await screen.findByRole("switch", { name: "Toggle host mode" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(
      screen.getByText("Configured mode: docker — shell commands run inside the sandbox."),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("OS account password")).not.toBeInTheDocument();
  });

  it("bật switch: hiện ô mật khẩu; Apply không nhập → lỗi os_password_required", async () => {
    const user = userEvent.setup();
    const scenario = hostModeHandlers({});
    testServer.use(...quietNeighbourHandlers, ...scenario.handlers);
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("switch", { name: "Toggle host mode" }));
    expect(screen.getByLabelText("OS account password")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("cần mật khẩu máy để bật/tắt host mode");
    // Chưa ghi được gì: trạng thái docker vẫn nguyên, form vẫn mở chờ nhập.
    expect(
      screen.getByText("Configured mode: docker — shell commands run inside the sandbox."),
    ).toBeInTheDocument();
    expect(scenario.state.posted).toEqual({ enabled: true, os_password: null });
  });

  it("sai mật khẩu: hiện lỗi từ server, không đổi trạng thái thành công", async () => {
    const user = userEvent.setup();
    const scenario = hostModeHandlers({ correctPassword: OS_PASSWORD });
    testServer.use(...quietNeighbourHandlers, ...scenario.handlers);
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("switch", { name: "Toggle host mode" }));
    await user.type(screen.getByLabelText("OS account password"), "wrong password entirely");
    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("mật khẩu máy không đúng");
    expect(screen.getByLabelText("OS account password")).toBeInTheDocument();
    expect(
      screen.getByText("Configured mode: docker — shell commands run inside the sandbox."),
    ).toBeInTheDocument();
  });

  it("đúng mật khẩu: ghi host vào đĩa, form đóng, hiện gợi ý restart", async () => {
    const user = userEvent.setup();
    const scenario = hostModeHandlers({ correctPassword: OS_PASSWORD });
    testServer.use(...quietNeighbourHandlers, ...scenario.handlers);
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("switch", { name: "Toggle host mode" }));
    await user.type(screen.getByLabelText("OS account password"), OS_PASSWORD);
    await user.click(screen.getByRole("button", { name: "Apply" }));

    // Sau thành công: ô mật khẩu đóng lại (bản nháp reset) và invalidate
    // hostMode đã refetch — GET trả mode mới + pending ⇒ gợi ý restart.
    await waitFor(() => {
      expect(screen.queryByLabelText("OS account password")).not.toBeInTheDocument();
    });
    expect(
      await screen.findByText(
        "Configured mode: host — shell commands run directly on this machine (no sandbox).",
      ),
    ).toBeInTheDocument();
    expect(
      await screen.findByText(
        "Configuration saved; restart Bean (scripts/beanctl restart) to make the new mode effective.",
      ),
    ).toBeInTheDocument();
  });

  it("máy không cần mật khẩu: không hiện ô mật khẩu, Apply gửi POST không có os_password", async () => {
    const user = userEvent.setup();
    const scenario = hostModeHandlers({ osPasswordRequired: false });
    testServer.use(...quietNeighbourHandlers, ...scenario.handlers);
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("switch", { name: "Toggle host mode" }));
    expect(screen.queryByLabelText("OS account password")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Apply" }));

    expect(scenario.state.posted).toEqual({ enabled: true, os_password: null });
    expect(
      await screen.findByText(
        "Configured mode: host — shell commands run directly on this machine (no sandbox).",
      ),
    ).toBeInTheDocument();
  });
});
