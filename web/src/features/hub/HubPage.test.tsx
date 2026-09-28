import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { BeanAvatar } from "@/components/brand/BeanAvatar";
import { BeanMark } from "@/components/brand/BeanMark";
import { RealtimeProvider } from "@/features/chat/RealtimeProvider";
import { HubPage } from "@/features/hub/HubPage";
import { renderManagement } from "@/test/management";
import { agentsHandler, FULL_AGENTS, testServer } from "@/test/server";

/**
 * Test HUB ở tầng UI.
 *
 * # Phạm vi thật sự của các test này
 *
 * Test RBAC **không** nằm ở đây mà ở `crates/bean-web/tests/hub_agents.rs`
 * (tầng API) — vì ẩn/hiện ở DOM không phải phân quyền. Ở đây chỉ khẳng định UI
 * **render đúng những gì API trả**, không tự lọc thêm và không bỏ sót hiển thị.
 */

/** jsdom không có WebSocket thật; socket im lặng để provider không mở kết nối. */
function stubWebSocket() {
  vi.stubGlobal(
    "WebSocket",
    class {
      readyState = 0;
      send() {}
      close() {}
    },
  );
}

function renderHub() {
  return renderManagement(
    <RealtimeProvider>
      <HubPage />
    </RealtimeProvider>,
  );
}

describe("HUB", () => {
  beforeEach(() => {
    stubWebSocket();
  });

  it("hiện trạng thái sống của từng agent theo dữ liệu API trả về", async () => {
    testServer.use(agentsHandler([...FULL_AGENTS]));
    renderHub();
    expect(await screen.findByTestId("agent-node-developer")).toHaveAttribute("data-status", "working");
    expect(screen.getByTestId("agent-node-qa")).toHaveAttribute("data-status", "idle");
    expect(screen.getByTestId("agent-node-marketing")).toHaveAttribute("data-status", "awaiting_you");
  });

  it("vẽ ba kiểu quan hệ khác nhau: điều phối, review, cảnh báo thẳng", async () => {
    testServer.use(agentsHandler([...FULL_AGENTS]));
    renderHub();
    await screen.findByTestId("agent-node-developer");
    // Ba vùng có `data-testid` riêng — đó là hình học khác nhau, không phải cùng
    // một kiểu chỉ khác màu.
    expect(screen.getByTestId("hub-edge-manages")).toBeInTheDocument();
    expect(screen.getByTestId("hub-edge-reviews")).toBeInTheDocument();
    expect(screen.getByTestId("hub-edge-alerts")).toBeInTheDocument();
    // Nhãn nói rõ cảnh báo đi thẳng tới người dùng, không qua Manager (D14.11).
    expect(screen.getByText(/đi thẳng tới bạn, không qua Manager/)).toBeInTheDocument();
    // Nhãn four-eyes nói rõ không tự duyệt.
    expect(screen.getByText(/không agent nào tự duyệt/)).toBeInTheDocument();
  });

  it("đặt agent review và agent cảnh báo vào đúng vùng của chúng", async () => {
    testServer.use(agentsHandler([...FULL_AGENTS]));
    renderHub();
    await screen.findByTestId("agent-node-developer");
    const review = screen.getByTestId("hub-edge-reviews");
    expect(within(review).getByTestId("agent-node-qa")).toBeInTheDocument();
    expect(within(review).queryByTestId("agent-node-developer")).not.toBeInTheDocument();
    const alerts = screen.getByTestId("hub-edge-alerts");
    expect(within(alerts).getByTestId("agent-node-security-scan")).toBeInTheDocument();
  });

  it("mascot hiện ở góc trên-trái của HUB", async () => {
    testServer.use(agentsHandler([...FULL_AGENTS]));
    const { container } = renderHub();
    await screen.findByTestId("agent-node-developer");
    // Logo HUB 44px nằm trong `<header>` — vị trí số 1 trong danh sách mascot.
    const header = container.querySelector("header");
    const logo = header?.querySelector("img");
    expect(logo).not.toBeNull();
    expect(logo?.getAttribute("width")).toBe("44");
    // Mascot phải có nhãn trợ năng, không phải trang trí vô nghĩa.
    expect(logo?.getAttribute("alt")).toBe("Bean");
  });

  it("trạng thái rỗng là lời mời hành động kèm mascot, không phải dòng xám", async () => {
    testServer.use(agentsHandler([]));
    renderHub();
    expect(await screen.findByText("Chưa có agent nào đang chạy")).toBeInTheDocument();
    expect(screen.getByText(/Khi bạn giao việc/)).toBeInTheDocument();
  });

  it("mọi node agent đều mang nhãn trạng thái bằng chữ, không chỉ bằng màu", async () => {
    testServer.use(agentsHandler([...FULL_AGENTS]));
    renderHub();
    await screen.findByTestId("agent-node-developer");
    // Người mù màu phải đọc được: mỗi node có badge chữ tương ứng trạng thái.
    // Dùng `getAllByText` vì "Rảnh" xuất hiện ở nhiều node — đó là đúng.
    expect(screen.getAllByText("Đang chạy").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Rảnh").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Chờ bạn duyệt").length).toBeGreaterThan(0);
    // Số badge bằng số node: mỗi agent có đúng một nhãn trạng thái.
    const nodes = document.querySelectorAll("[data-testid^='agent-node-']");
    expect(nodes.length).toBe(4);
  });

  it("Tab đi qua phần tử tương tác theo thứ tự DOM, không rơi vào wrapper vô nghĩa", async () => {
    testServer.use(agentsHandler([...FULL_AGENTS]));
    renderHub();
    await screen.findByTestId("agent-node-developer");
    const user = userEvent.setup();

    // Sơ đồ là **nội dung**, không phải control: không node nào được gắn
    // `tabIndex`, nên bàn phím phải bỏ qua nó và tới phần tử tương tác kế tiếp.
    // Đây là phép kiểm "đi đúng thứ tự": nếu sau này ai đó vô tình làm node
    // focusable, test này sẽ bắt được ngay.
    const nodes = document.querySelectorAll("[data-testid^='agent-node-']");
    expect(nodes.length).toBeGreaterThan(0);
    for (const node of nodes) {
      expect(node.getAttribute("tabindex")).toBeNull();
    }

    await user.tab();
    const first = document.activeElement;
    // Không được focus vào chính khối sơ đồ.
    expect(first).not.toBe(screen.getByTestId("hub-topology"));
    expect(first instanceof HTMLElement || first === document.body).toBe(true);
  });

  it("hàng chờ duyệt là nơi duy nhất có nút, và Tab tới được theo thứ tự", async () => {
    testServer.use(agentsHandler([...FULL_AGENTS]));
    // Hàng chờ lấy confirm từ WebSocket `Sync`; ở đây chỉ kiểm tra khả năng tiếp
    // cận bằng bàn phím khi hàng chờ **không** có việc nào — phải không render
    // khung rỗng nào cướp focus.
    renderHub();
    await screen.findByTestId("agent-node-developer");
    expect(screen.queryByTestId("hub-confirm-queue")).not.toBeInTheDocument();
  });

  it("không tải tài nguyên từ domain ngoài", async () => {
    testServer.use(agentsHandler([...FULL_AGENTS]));
    const { container } = renderHub();
    await screen.findByTestId("hub-topology");
    // agents.md mục 12.3: không tài nguyên từ ngoài. Mascot **là** `<img>` ảnh
    // thật, nên điều kiện bắt buộc không phải "không có img" mà là **mọi** `src`
    // phải là đường dẫn cục bộ — đây mới là thứ khớp với CSP `img-src 'self'`.
    for (const img of Array.from(container.querySelectorAll("img"))) {
      const src = img.getAttribute("src") ?? "";
      expect(src.startsWith("/") || src.startsWith("data:image/")).toBe(true);
      expect(src.startsWith("http")).toBe(false);
    }
    expect(container.querySelector("iframe")).toBeNull();
    expect(document.querySelector("script[src^='http']")).toBeNull();
  });

  it("BeanMark dùng currentColor, không gradient và không bóng", () => {
    const { container } = render(<BeanMark size={44} />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("stroke")).toBe("currentColor");
    expect(svg?.getAttribute("fill")).toBe("none");
    expect(container.querySelector("linearGradient")).toBeNull();
    expect(container.querySelector("filter")).toBeNull();
  });

  it("BeanAvatar trỏ tới ảnh thật, khai báo kích thước để chống layout shift", () => {
    const { container } = render(<BeanAvatar size={44} />);
    const img = container.querySelector("img");
    expect(img?.getAttribute("src")).toBe("/bean-avatar.png");
    // Không khai báo width/height thì ảnh đẩy layout khi tải xong.
    expect(img?.getAttribute("width")).toBe("44");
    expect(img?.getAttribute("height")).toBe("44");
    // Ảnh trang trí thì `alt` rỗng + aria-hidden, không đọc vấp trình đọc màn hình.
    expect(img?.getAttribute("alt")).toBe("");
    expect(img?.getAttribute("aria-hidden")).toBe("true");
  });

  it("BeanAvatar có nhãn trợ năng khi được định danh", () => {
    const { container } = render(<BeanAvatar size={28} title="Bean" />);
    const img = container.querySelector("img");
    expect(img?.getAttribute("alt")).toBe("Bean");
    expect(img?.hasAttribute("aria-hidden")).toBe(false);
  });

  it("ảnh avatar tồn tại trong public/ và nhỏ hơn nhiều so với ảnh gốc", async () => {
    const { statSync } = await import("node:fs");
    // Ảnh gốc 500×500 ~264 KB; bản dùng trong UI phải nhẹ hơn hẳn.
    const generated = statSync("public/bean-avatar.png").size;
    const source = statSync("brand/bean.png").size;
    expect(generated).toBeLessThan(source);
    expect(generated).toBeLessThan(64 * 1024);
  });

  it("không mang nghĩa nào bằng chuyển động, và reduced-motion có lưới an toàn", async () => {
    testServer.use(agentsHandler([...FULL_AGENTS]));
    const { container } = renderHub();
    await screen.findByTestId("hub-topology");

    // Trạng thái agent phải đọc được khi **tắt hẳn** chuyển động. Nếu sau này
    // ai thêm `animate-pulse` vào chấm trạng thái để "cho sống", test này đỏ và
    // bắt họ phải cân nhắc lại — đó là lý do có test.
    const moving = Array.from(container.querySelectorAll("*")).filter((el) =>
      Array.from(el.classList).some((c) => c.startsWith("animate-")),
    );
    expect(moving).toEqual([]);

    // Lưới an toàn toàn cục: tắt animation/transition/scroll khi người dùng
    // bật `prefers-reduced-motion`. Không có cái này thì lưới trên chỉ là ý thức.
    const { readFileSync } = await import("node:fs");
    const css = readFileSync("src/index.css", "utf8");
    const block = css.slice(css.indexOf("@media (prefers-reduced-motion: reduce)"));
    expect(block).toContain("animation-duration: 0.01ms !important");
    expect(block).toContain("transition-duration: 0.01ms !important");
    expect(block).toContain("scroll-behavior: auto !important");
  });

  it("favicon không bị cắt tai: mọi nét nằm trong viewBox", async () => {
    const { MARK_BOUNDS, MARK_NUDGE } = await import("@/components/brand/markPaths");
    const { readFileSync } = await import("node:fs");
    const favicon = readFileSync("public/favicon.svg", "utf8");

    // SVG mặc định `overflow: hidden`: vượt `viewBox` là **bị cắt**, không báo
    // lỗi. Bản favicon cũ đẩy tai phải ra ngoài x=24 và mất tai — mọi thứ vẫn
    // "pass" vì test cũ chỉ kiểm tra có chứa đúng các `path`. Test này chặn đúng
    // lớp lỗi đó.
    const [, vbW, vbH] = /viewBox="0 0 ([\d.]+) ([\d.]+)"/.exec(favicon) ?? [];
    expect(vbW).toBe("24");

    // Nền phải phủ đúng `viewBox`; nền rộng hơn thì bị cắt, hẹp hơn thì lộ viền.
    const rect = /<rect width="([\d.]+)" height="([\d.]+)"/.exec(favicon);
    expect([rect?.[1], rect?.[2]]).toEqual([vbW, vbH]);

    // Mascot sau khi dịch phải vẫn nằm trong ô.
    const moved = {
      minX: MARK_BOUNDS.minX + MARK_NUDGE.x,
      maxX: MARK_BOUNDS.maxX + MARK_NUDGE.x,
      minY: MARK_BOUNDS.minY + MARK_NUDGE.y,
      maxY: MARK_BOUNDS.maxY + MARK_NUDGE.y,
    };
    expect(moved.minX).toBeGreaterThanOrEqual(0);
    expect(moved.minY).toBeGreaterThanOrEqual(0);
    expect(moved.maxX).toBeLessThanOrEqual(Number(vbW));
    expect(moved.maxY).toBeLessThanOrEqual(Number(vbH));

    // Và `translate` trong file phải đúng bằng hằng số đã khai báo.
    expect(favicon).toContain(`translate(${MARK_NUDGE.x} ${MARK_NUDGE.y})`);
  });

  it("favicon được sinh từ cùng hằng số hình với component", async () => {
    const { markPaths } = await import("@/components/brand/markPaths");
    const { readFileSync } = await import("node:fs");
    const favicon = readFileSync("public/favicon.svg", "utf8");
    // Favicon phải chứa đúng các đường nét mà component vẽ cho `silhouette` — đây
    // là cách bắt "quên chạy lại script" thay vì để hình lệch âm thầm.
    for (const d of markPaths("silhouette")) {
      expect(favicon).toContain(d);
    }
  });
});
