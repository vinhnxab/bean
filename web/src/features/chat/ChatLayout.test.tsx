import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import type { ReactNode } from "react";
import { Route, Routes } from "react-router";
import { beforeEach, describe, expect, it } from "vitest";

import type { SessionDto } from "@/api/bindings";
import { ChatLayout } from "@/features/chat/ChatLayout";
import { SIDEBAR_COLLAPSED_KEY } from "@/lib/sidebar";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const session: SessionDto = {
  id: 4,
  channel: "web",
  chat_id: "web:admin",
  user_id: "web:admin",
  title: "Ke hoach hom nay",
  archived: false,
  created_at: "2026-09-26T00:00:00Z",
  updated_at: "2026-09-26T00:00:00Z",
};

function renderSidebar() {
  const user = userEvent.setup();
  renderManagement(
    <Routes>
      <Route element={<ChatLayout />}>
        <Route path="/" element={<div>hub</div>} />
        <Route path="/sessions/:sessionId" element={<div>chat</div>} />
      </Route>
    </Routes>,
    ["/sessions/4"],
  );
  return user;
}

describe("ChatLayout sidebar", () => {
  // Mọi test trong file đều phải bắt đầu từ trạng thái sidebar **mở rộng**.
  // `useSidebarCollapsed` đọc `localStorage`, nên lựa chọn của test trước sẽ rò
  // sang test sau và làm chúng fail tuỳ thứ tự chạy — cùng đúng cái
  // `theme.test.tsx` đã làm. Xoá ở cấp `describe` để không sót test nào.
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("tim hoi thoai theo tieu de va bao ro khi khong co ket qua", async () => {
    testServer.use(
      http.get("/api/sessions", ({ request }) => {
        const query = new URL(request.url).searchParams.get("q");
        return HttpResponse.json({ sessions: query ? [] : [session] });
      }),
    );
    const user = renderSidebar();
    await screen.findByText("Ke hoach hom nay");
    await user.type(screen.getByLabelText("Search conversations"), "khong co");
    await screen.findByText("No conversations found.");
  }, 15000);

  it("gom hoi thoai theo moc thoi gian", async () => {
    const today = new Date();
    const older = new Date(today.getTime() - 20 * 86400000);
    testServer.use(
      http.get("/api/sessions", () =>
        HttpResponse.json({
          sessions: [
            { ...session, id: 10, title: "Ke hoach buoi sang", updated_at: today.toISOString() },
            { ...session, id: 11, title: "Du an cu", updated_at: older.toISOString() },
          ],
        }),
      ),
    );
    renderSidebar();
    await screen.findByText("Ke hoach buoi sang");
    await screen.findByText("Du an cu");
    expect(screen.getByRole("heading", { name: "Today" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Previous 30 days" })).toBeInTheDocument();
  }, 15000);

  it("điều hướng tới màn Tools và MCP", async () => {
    renderSidebar();
    expect(await screen.findByRole("link", { name: /Tools/ })).toHaveAttribute("href", "/tools");
    expect(screen.getByRole("link", { name: /MCP/ })).toHaveAttribute("href", "/mcp");
  });

  // Cài đặt là mục "hệ thống" nằm ở **footer** sidebar, tách khỏi điều hướng
  // chính — chat thường xuyên dùng nhất, cài đặt chỉ cần luôn biết tìm ở đáy.
  it("Cài đặt nằm ở footer sidebar và trỏ /settings", async () => {
    renderSidebar();
    const settings = await screen.findByRole("link", { name: "Settings" });
    expect(settings).toHaveAttribute("href", "/settings");
    // Footer nằm sau nav chính trong DOM (tách khỏi NAV_ITEMS).
    const nav = screen.getByRole("navigation", { name: "Main navigation" });
    expect(nav.contains(settings)).toBe(false);
  });

  // Sidebar phải ở màn Cài đặt là **nav cài đặt**, không phải danh sách hội
  // thoại: hai ngăn kéo không cùng tồn tại trên cùng một màn.
  it("ở /settings, sidebar phải là nav cài đặt thay cho danh sách hội thoại", async () => {
    renderManagement(
      <Routes>
        <Route element={<ChatLayout />}>
          <Route path="/settings/:section" element={<div>providers</div>} />
        </Route>
      </Routes>,
      ["/settings/providers"],
    );

    await screen.findByText("providers");
    expect(screen.getByTestId("settings-sidebar")).toBeInTheDocument();
    expect(screen.queryByTestId("chat-sidebar")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Providers/ })).toHaveAttribute("href", "/settings/providers");
  }, 15000);

  it("trang chu HUB khong loi danh sach hoi thoai vao", async () => {
    testServer.use(http.get("/api/sessions", () => HttpResponse.json({ sessions: [session] })));
    renderManagement(
      <Routes>
        <Route element={<ChatLayout />}>
          <Route path="/" element={<div>hub</div>} />
        </Route>
      </Routes>,
    );
    await screen.findByText("hub");
    expect(screen.queryByLabelText("Search conversations")).not.toBeInTheDocument();
    // HUB không có ngữ cảnh hội thoại: sidebar phải không render, và TopNav
    // không có nút mở ngăn kéo hội thoại.
    expect(screen.queryByTestId("chat-sidebar")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open conversation list" })).not.toBeInTheDocument();
  }, 15000);

  // Mở bằng **bàn phím** chứ không bằng `user.click`: Radix bọc nội dung trong
  // `DismissableLayer` ở phạm vi module, nên cú click chuỗi-pointer ở test sau có
  // thể bị bỏ qua (triệu chứng: `data-state` vẫn `closed`, lỗi báo sai lệch
  // "không tìm thấy option"). Xem giải thích dài hơn ở `SessionRow.test.tsx`.
  it("đổi ngôn ngữ bằng Select, không phải bằng `<select>` native", async () => {
    const user = userEvent.setup();
    renderSidebar();

    // Nút cho biết ngôn ngữ đang dùng ngay, không bắt mở ra mới biết.
    // Mặc định là tiếng Anh nên nhãn là "Language: EN".
    const trigger = await screen.findByRole("combobox", { name: "Language: EN" });
    // `<select>` native mở ra là giao diện của hệ điều hành: không nhận token màu
    // của app. Đây là lý do nó không được dùng ở bất kỳ đâu trong UI.
    expect(document.querySelector("select")).toBeNull();

    trigger.focus();
    await user.keyboard("{Enter}");
    // `Select` ⇒ mục mang `role="option"` + `aria-selected`, nên trình đọc màn
    // hình đọc được *đang chọn* chứ không chỉ biết có một mục tên như vậy.
    // `DropdownMenu` (`menuitemradio`) là sai ở đây: ngôn ngữ là trạng thái,
    // không phải hành động.
    const enOption = await screen.findByRole("option", { name: "EN" });
    expect(enOption).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("menuitemradio")).not.toBeInTheDocument();

    screen.getByRole("option", { name: "VI" }).focus();
    await user.keyboard("{Enter}");
    // Chọn xong danh sách đóng và nút cập nật nhãn.
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(await screen.findByRole("combobox", { name: "Ngôn ngữ: VI" })).toBeInTheDocument();
  }, 15000);

  // `Suspense` nằm quanh `<Outlet />` chứ không quanh cả `ChatLayout` (xem `App.tsx`).
  // Nếu ai đó chuyển nó ra ngoài, lúc chunk đang tải **cả khung** biến mất: sidebar
  // nhấp nháy, người dùng mất điểm neo đang bấm. Test này chốt đúng ranh giới đó.
  it("route đang treo thì sidebar vẫn còn, chỉ vùng nội dung là skeleton", async () => {
    // Kiểu trả về phải là `ReactNode`: hàm chỉ ném promise nên suy ra `void`, mà
    // `() => void` không phải component hợp lệ với React 19.
    function Never(): ReactNode {
      // Tương đương chunk chưa tải xong: ném promise rồi không bao giờ resolve.
      throw new Promise<void>(() => {});
    }
    renderManagement(
      <Routes>
        <Route element={<ChatLayout />}>
          <Route path="/" element={<Never />} />
        </Route>
      </Routes>,
    );
    // Skeleton nội dung: `role="status"` do `RouteFallback`.
    await screen.findByRole("status");
    // Sidebar trái vẫn hiện đầy đủ — đây mới là điều cần giữ. Nhãn lấy đúng từ
    // từ điển (`common.account`) để đổi copy không làm test hỏng theo. Đăng xuất
    // nằm trong menu tài khoản của TopNav, chưa mở menu thì chỉ có nút avatar.
    expect(screen.getByRole("link", { name: /Tools/ })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /MCP/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Account" })).toBeInTheDocument();
  });

  // Ba tính năng sidebar thêm ở đợt sửa giao diện: thu gọn được, mục điều
  // hướng có icon thật, và ngôn ngữ + sáng/tối nằm chung một hàng.
  describe("thu gọn sidebar", () => {
    // `localStorage` đã được xoá ở `beforeEach` cấp `describe` cha.
    it("mặc định mở rộng, bấm nút thì thu gọn và nhớ lựa chọn", async () => {
      const user = renderSidebar();
      const toggle = await screen.findByRole("button", { name: "Collapse sidebar" });
      // Nhãn mô tả hành động **sắp làm**; `aria-expanded` mô tả trạng thái hiện tại.
      expect(toggle).toHaveAttribute("aria-expanded", "true");

      await user.click(toggle);

      // Sau khi thu gọn: nhãn đảo chiều (giờ hành động là mở rộng) và
      // `aria-expanded` thành false.
      const expand = screen.getByRole("button", { name: "Expand sidebar" });
      expect(expand).toHaveAttribute("aria-expanded", "false");
      expect(screen.getByTestId("sidebar-shell")).toHaveClass("md:w-20");

      // Lựa chọn được ghi lại: tải lại trang không bật lại thành mở rộng.
      expect(window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY)).toBe("1");

      await user.click(expand);
      expect(screen.getByRole("button", { name: "Collapse sidebar" })).toHaveAttribute(
        "aria-expanded",
        "true",
      );
      expect(screen.getByTestId("sidebar-shell")).toHaveClass("md:w-72");
    }, 15000);

    it("khi thu gọn, nhãn mục điều hướng vẫn còn cho trình đọc màn hình", async () => {
      const user = renderSidebar();
      await user.click(await screen.findByRole("button", { name: "Collapse sidebar" }));

      // Nhãn chuyển sang `sr-only` chứ không bị xoá: mục điều hướng trong rail
      // phải vẫn tên cho người dùng đọc bằng trình đọc màn hình, nếu không rail
      // chỉ là một đống icon vô danh.
      const link = screen.getByRole("link", { name: "Chat" });
      expect(link).toHaveAttribute("href", "/chat");
      // Tooltip là gợi ý cho con trỏ, nên nhãn phải nằm trong chính link.
      expect(link.querySelector(".sr-only")).toHaveTextContent("Chat");
    }, 15000);

    it("nút hội thoại mới nằm ở sidebar phải, không phụ thuộc rail trái", async () => {
      const user = renderSidebar();
      await user.click(await screen.findByRole("button", { name: "Collapse sidebar" }));

      // Việc tạo hội thoại thuộc sidebar phải; rail trái thu gọn không có nghĩa
      // là mất nút — hai sidebar độc lập với nhau.
      expect(screen.getByRole("button", { name: "New conversation" })).toBeInTheDocument();
    }, 15000);

    // Ngăn kéo hội thoại là câu trả lời cho "trên điện thoại thì đổi hội thoại
    // bằng cách nào": nút mở nằm trên TopNav, kéo từ mép phải, có overlay.
    describe("ngăn kéo hội thoại (mobile)", () => {
      const drawer = () => screen.getByTestId("chat-sidebar");

      it("bấm icon trên TopNav thì danh sách hiện ra kèm overlay", async () => {
        testServer.use(http.get("/api/sessions", () => HttpResponse.json({ sessions: [session] })));
        const user = renderSidebar();
        const toggle = await screen.findByRole("button", { name: "Open conversation list" });
        // Chưa mở: ngăn kéo không mang cờ mở (desktop thì luôn mở, không cần cờ).
        expect(drawer()).not.toHaveAttribute("data-open");

        await user.click(toggle);

        expect(drawer()).toHaveAttribute("data-open");
        expect(await screen.findByText("Ke hoach hom nay")).toBeInTheDocument();
        // Ô tìm kiếm phải có thật: cần lọc danh sách dài ngay trong ngăn kéo.
        expect(screen.getByLabelText("Search conversations")).toBeInTheDocument();
        // Overlay mờ nằm ngay trước ngăn kéo trong DOM, là `<button>` để bấm ra
        // ngoài đóng được bằng bàn phím.
        const overlay = drawer().previousElementSibling;
        expect(overlay).toHaveAttribute("aria-label", "Close");
      }, 15000);

      it("đóng bằng nút X trong đầu ngăn kéo", async () => {
        testServer.use(http.get("/api/sessions", () => HttpResponse.json({ sessions: [session] })));
        const user = renderSidebar();
        await user.click(await screen.findByRole("button", { name: "Open conversation list" }));
        await screen.findByText("Ke hoach hom nay");

        // Nút X nằm trong chính ngăn kéo — `within` để không bấm nhầm nút "Đóng"
        // của ngăn kéo điều hướng bên trái hay của overlay.
        await user.click(within(drawer()).getByRole("button", { name: "Close" }));

        await waitFor(() => expect(drawer()).not.toHaveAttribute("data-open"));
        // Overlay biến mất theo ngăn kéo.
        await waitFor(() => expect(drawer().previousElementSibling).not.toHaveAttribute("aria-label"));
      }, 15000);
    });
  });

  // TopNav là nơi duy nhất chứa lựa chọn toàn cục: chủ đề và tài khoản. Đăng
  // xuất phải nằm trong menu tài khoản — không bấm nhầm được khi chưa mở.
  it("TopNav: chủ đề nằm trên thanh trên cùng, đăng xuất nằm trong menu tài khoản", async () => {
    const user = userEvent.setup();
    renderSidebar();

    // Nút chủ đề là icon trơn: tên trợ năng là toàn bộ hành động, không có nhãn chữ.
    // Mặc định là tối nên hành động là chuyển sang sáng.
    const theme = screen.getByRole("button", { name: /Switch to light/ });
    expect(theme).toHaveAccessibleName(/Switch to light/);

    // Chưa mở menu thì không có nút đăng xuất nào ngoài đó. Mở bằng bàn phím:
    // giống test ngôn ngữ ở trên, cú click chuỗi-pointer trong file dài có thể
    // bị bỏ qua (xem `SessionRow.test.tsx`), còn `focus` + `Enter` là đường
    // ký tự mà Radix luôn lắng nghe.
    expect(screen.queryByRole("button", { name: "Sign out" })).not.toBeInTheDocument();
    const account = await screen.findByRole("button", { name: "Account" });
    account.focus();
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("menuitem", { name: "Sign out" })).toBeInTheDocument();
  }, 15000);

  it("thu gọn: nút mở rộng sidebar nằm giữa rail", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Collapse sidebar" }));

    const expand = screen.getByRole("button", { name: "Expand sidebar" });
    // Header chỉ còn một nút khi brand chuyển lên TopNav; rail phải giữ cảm giác
    // cột icon nên nút nằm giữa.
    expect(expand.parentElement).toHaveClass("justify-center");
  }, 15000);

  it("mục điều hướng dùng icon vector thay cho ký tự Unicode", async () => {
    renderSidebar();
    const link = await screen.findByRole("link", { name: "Chat" });

    // Icon `<svg>` của lucide, không phải ký tự kiểu cũ (`◌`): ký tự Unicode đổi
    // hình dáng theo font của hệ điều hành, còn SVG vẽ bằng `currentColor`.
    const icon = link.querySelector("svg");
    expect(icon).not.toBeNull();
    expect(icon).toHaveAttribute("aria-hidden", "true");
    // Nhãn chữ vẫn là văn bản thật bên cạnh icon, không nằm trong `<title>`.
    expect(link.querySelector("span")).toHaveTextContent("Chat");
  }, 15000);

  // Hồi quy: `<TooltipTrigger asChild>` **clone** con rồi đè `className` của
  // trigger lên nó. Vì `NavItem` thu gọn nằm trong tooltip, `className` dạng hàm
  // của `NavLink` từng biến thành **chuỗi function chưa từng chạy** — mọi padding
  // và `size-9` mất sạch mà không có lỗi console nào.
  it("class của mục điều hướng được áp dụng thật khi thu gọn", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Collapse sidebar" }));

    const link = screen.getByRole("link", { name: "Chat" });
    const className = link.getAttribute("class") ?? "";

    // Chuỗi chứa `=>` nghĩa là `className` dạng hàm chưa được React gọi — đúng
    // triệu chứng hỏng: toàn bộ style trong `itemClass` biến mất.
    expect(className).not.toContain("=>");
    // `size-9` là khoảng chạm 36px của rail; mất nó thì mục chỉ còn 16px icon,
    // khoảng chạm quá nhỏ trên cảm ứng.
    expect(className).toContain("size-9");
  }, 15000);

  // Radix merge là **nối chuỗi** `childProps` + `triggerProps`, nên class bị lặp
  // hai lần. Lặp thì vô hại, nhưng chỉ `cn()` mới giữ được một cặp duy nhất —
  // và nếu hai bên rời nhau, class thừa sẽ thắng class đúng.
  it("class của mục không bị nhân đôi khi có tooltip", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Collapse sidebar" }));

    const link = screen.getByRole("link", { name: "Chat" });
    const matches = link.getAttribute("class")?.match(/size-9/g) ?? [];
    expect(matches).toHaveLength(1);
  }, 15000);

  // Mục đang active vẫn phải nổi bật sau khi bỏ `className` dạng hàm: `isActive`
  // giờ do `useMatch` tính, nên phải chốt lại để không mất dấu hiệu này vô hình.
  it("mục đang active vẫn được đánh dấu", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Collapse sidebar" }));

    // `renderSidebar()` mở route `/sessions/4`, nên **không** có mục nào active
    // với `end` — mọi mục phải mang class của trạng thái không active. Điều này
    // bắt được lỗi ngược lại: `isActive` luôn `true` sẽ làm mọi mục nổi bật.
    const chat = screen.getByRole("link", { name: "Chat" });
    expect(chat.className).not.toContain("bg-accent");
    expect(chat.className).toContain("text-muted-foreground");
  }, 15000);

  // `useMatch` phải theo route đang mở, không phải luôn trả về khớp.
  it("mục active đổi theo route", async () => {
    const user = userEvent.setup();
    renderManagement(
      <Routes>
        <Route element={<ChatLayout />}>
          <Route path="/" element={<div>hub</div>} />
          <Route path="/status" element={<div>status</div>} />
        </Route>
      </Routes>,
      ["/status"],
    );

    await user.click(await screen.findByRole("button", { name: "Collapse sidebar" }));
    expect(screen.getByRole("link", { name: "Status" }).className).toContain("bg-accent");
    expect(screen.getByRole("link", { name: "Hub" }).className).not.toContain("bg-accent");
  }, 15000);
});
