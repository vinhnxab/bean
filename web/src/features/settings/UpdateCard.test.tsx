import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import type { UpdateCheckDto } from "@/api/generated/UpdateCheckDto";
import { AdvancedPanel } from "@/features/settings/AdvancedPanel";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const RELEASE_URL = "https://github.com/bean-project/bean/releases/tag/v0.2.0";

const updateAvailable: UpdateCheckDto = {
  current_version: "0.1.0",
  latest_version: "v0.2.0",
  update_available: true,
  release_url: RELEASE_URL,
  notes: "",
  artifact: "bean-x86_64-unknown-linux-gnu.tar.gz",
};

/**
 * Panel Advanced mount cả card LAN — hai handler này giữ card hàng xóm im lặng
 * (không phát `alert` riêng) để `findByRole("alert")` chỉ trúng lỗi update check.
 * Host-mode GET đã có handler mặc định trong `test/server.ts` (như LanAccessCard.test).
 */
const quietNeighbourHandlers = [
  http.get("/api/settings/lan-access", () =>
    HttpResponse.json({ mode: "loopback", lan_ip: "192.168.1.29", port: 7878, url: null }),
  ),
  http.get("/api/settings", () => HttpResponse.json({ settings: [], writable: true })),
];

describe("UpdateCard — kiểm tra bản mới (M30)", () => {
  it("không tự gọi /api/update/check lúc mount (D23.6) — chỉ gọi khi bấm nút", async () => {
    let checkCalls = 0;
    testServer.use(
      ...quietNeighbourHandlers,
      http.get("/api/update/check", () => {
        checkCalls += 1;
        return HttpResponse.json(updateAvailable);
      }),
    );
    renderManagement(<AdvancedPanel />);

    // Panel đã mount ổn định (card hàng xóm load xong) mà chưa có request check nào.
    expect(await screen.findByRole("switch", { name: "Toggle LAN access" })).toBeInTheDocument();
    expect(checkCalls).toBe(0);
  });

  it("có bản mới: hiện status, link release mở tab mới và hint lệnh `bean update` dạng <code>", async () => {
    const user = userEvent.setup();
    testServer.use(
      ...quietNeighbourHandlers,
      http.get("/api/update/check", () => HttpResponse.json(updateAvailable)),
    );
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("button", { name: "Check for updates" }));

    const status = await screen.findByTestId("update-status");
    // Template i18n tự thêm tiền tố `v` cho cả hai placeholder; `latest_version`
    // là tag GitHub (đã có `v`) — assert interpolation, không ghim chuỗi ghép kép.
    expect(status).toHaveTextContent("New version available: v0.1.0");
    expect(status).toHaveTextContent(updateAvailable.latest_version);
    const link = screen.getByRole("link");
    expect(link).toHaveAttribute("href", RELEASE_URL);
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(screen.getByText("bean update").tagName).toBe("CODE");
  });

  it("chưa có bản mới: hiện trạng thái đang ở bản mới nhất, không có link release", async () => {
    const user = userEvent.setup();
    testServer.use(
      ...quietNeighbourHandlers,
      http.get("/api/update/check", () =>
        HttpResponse.json({ ...updateAvailable, update_available: false, latest_version: "v0.1.0" }),
      ),
    );
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("button", { name: "Check for updates" }));

    expect(await screen.findByTestId("update-status")).toHaveTextContent(
      "You are on the latest version (v0.1.0).",
    );
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });

  it("nền tảng không có artifact: hiện cảnh báo phải tải tay, status vẫn hiển thị", async () => {
    const user = userEvent.setup();
    testServer.use(
      ...quietNeighbourHandlers,
      http.get("/api/update/check", () => HttpResponse.json({ ...updateAvailable, artifact: null })),
    );
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("button", { name: "Check for updates" }));

    expect(
      await screen.findByText(
        "No build artifact exists for this server's platform yet — download one manually from the release page.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByTestId("update-status")).toHaveTextContent("New version available");
  });

  it("lỗi 502 update_check_failed: hiện message từ server qua role=alert, không hiện status", async () => {
    const user = userEvent.setup();
    testServer.use(
      ...quietNeighbourHandlers,
      http.get("/api/update/check", () =>
        HttpResponse.json({ code: "update_check_failed", message: "GitHub không phản hồi" }, { status: 502 }),
      ),
    );
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("button", { name: "Check for updates" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("GitHub không phản hồi");
    expect(screen.queryByTestId("update-status")).not.toBeInTheDocument();
  });

  it("notes không tin cậy: hiển thị thuần văn bản, không render HTML/script", async () => {
    const user = userEvent.setup();
    testServer.use(
      ...quietNeighbourHandlers,
      http.get("/api/update/check", () =>
        HttpResponse.json({
          ...updateAvailable,
          notes: "Patch notes <script>alert('pwned')</script> & more",
        }),
      ),
    );
    renderManagement(<AdvancedPanel />);

    await user.click(await screen.findByRole("button", { name: "Check for updates" }));

    // Nội dung HTML xuất hiện NGUYÊN VĂN dạng text (React escape), không có thẻ nào được tạo.
    expect(await screen.findByText(/<script>alert\('pwned'\)<\/script>/)).toBeInTheDocument();
    expect(document.querySelector("script")).toBeNull();
  });
});
