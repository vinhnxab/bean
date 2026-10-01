import { screen, waitFor } from "@testing-library/react";
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
    await user.type(screen.getByLabelText("Tìm hội thoại"), "khong co");
    await screen.findByText("Không tìm thấy hội thoại.");
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
    expect(screen.getByRole("heading", { name: "Hôm nay" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "30 ngày trước" })).toBeInTheDocument();
  }, 15000);

  it("điều hướng tới màn Tools và MCP", async () => {
    renderSidebar();
    expect(await screen.findByRole("link", { name: /Tools/ })).toHaveAttribute("href", "/tools");
    expect(screen.getByRole("link", { name: /MCP/ })).toHaveAttribute("href", "/mcp");
  });

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
    expect(screen.queryByLabelText("Tìm hội thoại")).not.toBeInTheDocument();
  }, 15000);

  // Mở bằng **bàn phím** chứ không bằng `user.click`: Radix bọc nội dung trong
  // `DismissableLayer` ở phạm vi module, nên cú click chuỗi-pointer ở test sau có
  // thể bị bỏ qua (triệu chứng: `data-state` vẫn `closed`, lỗi báo sai lệch
  // "không tìm thấy option"). Xem giải thích dài hơn ở `SessionRow.test.tsx`.
  it("đổi ngôn ngữ bằng Select, không phải bằng `<select>` native", async () => {
    const user = userEvent.setup();
    renderSidebar();

    // Nút cho biết ngôn ngữ đang dùng ngay, không bắt mở ra mới biết.
    const trigger = await screen.findByRole("combobox", { name: "Ngôn ngữ: Tiếng Việt" });
    // `<select>` native mở ra là giao diện của hệ điều hành: không nhận token màu
    // của app. Đây là lý do nó không được dùng ở bất kỳ đâu trong UI.
    expect(document.querySelector("select")).toBeNull();

    trigger.focus();
    await user.keyboard("{Enter}");
    // `Select` ⇒ mục mang `role="option"` + `aria-selected`, nên trình đọc màn
    // hình đọc được *đang chọn* chứ không chỉ biết có một mục tên như vậy.
    // `DropdownMenu` (`menuitemradio`) là sai ở đây: ngôn ngữ là trạng thái,
    // không phải hành động.
    const viOption = await screen.findByRole("option", { name: "Tiếng Việt" });
    expect(viOption).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("menuitemradio")).not.toBeInTheDocument();

    screen.getByRole("option", { name: "English" }).focus();
    await user.keyboard("{Enter}");
    // Chọn xong danh sách đóng và nút cập nật nhãn.
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(await screen.findByRole("combobox", { name: "Language: English" })).toBeInTheDocument();
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
    // Sidebar vẫn hiện đầy đủ — đây mới là điều cần giữ. Nhãn lấy đúng từ từ điển
    // (`chat.logout`) để đổi copy không làm test hỏng theo.
    expect(screen.getByRole("link", { name: /Tools/ })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /MCP/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Đăng xuất" })).toBeInTheDocument();
  });

  // Ba tính năng sidebar thêm ở đợt sửa giao diện: thu gọn được, mục điều
  // hướng có icon thật, và ngôn ngữ + sáng/tối nằm chung một hàng.
  describe("thu gọn sidebar", () => {
    // `localStorage` đã được xoá ở `beforeEach` cấp `describe` cha.
    it("mặc định mở rộng, bấm nút thì thu gọn và nhớ lựa chọn", async () => {
      const user = renderSidebar();
      const toggle = await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" });
      // Nhãn mô tả hành động **sắp làm**; `aria-expanded` mô tả trạng thái hiện tại.
      expect(toggle).toHaveAttribute("aria-expanded", "true");

      await user.click(toggle);

      // Sau khi thu gọn: nhãn đảo chiều (giờ hành động là mở rộng) và
      // `aria-expanded` thành false.
      const expand = screen.getByRole("button", { name: "Mở rộng thanh điều hướng" });
      expect(expand).toHaveAttribute("aria-expanded", "false");
      expect(screen.getByTestId("sidebar-shell")).toHaveClass("md:w-20");

      // Lựa chọn được ghi lại: tải lại trang không bật lại thành mở rộng.
      expect(window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY)).toBe("1");

      await user.click(expand);
      expect(screen.getByRole("button", { name: "Thu gọn thanh điều hướng" })).toHaveAttribute(
        "aria-expanded",
        "true",
      );
      expect(screen.getByTestId("sidebar-shell")).toHaveClass("md:w-72");
    }, 15000);

    it("khi thu gọn, nhãn mục điều hướng vẫn còn cho trình đọc màn hình", async () => {
      const user = renderSidebar();
      await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

      // Nhãn chuyển sang `sr-only` chứ không bị xoá: mục điều hướng trong rail
      // phải vẫn tên cho người dùng đọc bằng trình đọc màn hình, nếu không rail
      // chỉ là một đống icon vô danh.
      const link = screen.getByRole("link", { name: "Trò chuyện" });
      expect(link).toHaveAttribute("href", "/chat");
      // Tooltip là gợi ý cho con trỏ, nên nhãn phải nằm trong chính link.
      expect(link.querySelector(".sr-only")).toHaveTextContent("Trò chuyện");
    }, 15000);

    it("nút hội thoại mới còn dùng được khi thu gọn", async () => {
      const user = renderSidebar();
      await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

      // Rail giữ lại hành động quan trọng nhất: tạo hội thoại mới, làm được
      // chỉ với một icon.
      const create = screen.getByRole("button", { name: "Hội thoại mới" });
      expect(create).toBeInTheDocument();
      expect(create.querySelector(".sr-only")).toHaveTextContent("Hội thoại mới");
    }, 15000);

    // Flyout là câu trả lời cho "rail 80px thì đổi hội thoại bằng cách nào".
    // Ba test dưới chốt đúng ba điều: mở được, **không chặn** nội dung chat, và
    // đóng đúng cách.
    describe("flyout danh sách hội thoại", () => {
      it("chưa thu gọn thì không có nút lịch sử", async () => {
        // Handler mặc định của `test/server.ts` trả `sessions: []`, nên phải mock
        // riêng — không thì `findByText` chờ một danh sách vốn rỗng và test fail
        // với lý do sai.
        testServer.use(http.get("/api/sessions", () => HttpResponse.json({ sessions: [session] })));
        renderSidebar();
        // Danh sách đã nằm sẵn trong sidebar mở rộng; một nút mở lại nó là thừa.
        await screen.findByText("Ke hoach hom nay");
        expect(screen.queryByRole("button", { name: "Mở danh sách hội thoại" })).not.toBeInTheDocument();
      }, 15000);

      it("thu gọn rồi bấm icon lịch sử thì danh sách hiện ra", async () => {
        testServer.use(http.get("/api/sessions", () => HttpResponse.json({ sessions: [session] })));
        const user = renderSidebar();
        await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

        const toggle = screen.getByRole("button", { name: "Mở danh sách hội thoại" });
        expect(toggle).toHaveAttribute("aria-expanded", "false");
        // Rail không chứa nổi danh sách nên trước khi bấm, hội thoại chưa hiện.
        expect(screen.queryByText("Ke hoach hom nay")).not.toBeInTheDocument();

        await user.click(toggle);

        const flyout = await screen.findByRole("dialog");
        expect(flyout).toHaveAttribute("data-slot", "history-flyout");
        expect(await screen.findByText("Ke hoach hom nay")).toBeInTheDocument();
        // Ô tìm kiếm phải có thật: đây là lý do flyout hơn modal — cần lọc danh
        // sách dài ngay trong bảng.
        expect(screen.getByLabelText("Tìm hội thoại")).toBeInTheDocument();
      }, 15000);

      it("không có overlay tối: nội dung chat phía sau vẫn thấy", async () => {
        const user = renderSidebar();
        await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));
        await user.click(screen.getByRole("button", { name: "Mở danh sách hội thoại" }));
        await screen.findByRole("dialog");

        // `modal={false}` ⇒ Radix không render `DialogOverlay`. Đây là điểm cốt
        // lõi của thiết kế: modal sẽ che mất cuộc hội thoại đang đọc.
        expect(document.querySelector("[data-slot='dialog-overlay']")).toBeNull();
        // Vùng nội dung vẫn trong cây DOM, tức là vẫn nhìn thấy và đọc được.
        expect(screen.getByText("chat")).toBeInTheDocument();
      }, 15000);

      it("đóng bằng Esc và đóng tự động khi bung sidebar ra", async () => {
        const user = renderSidebar();
        await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));
        const toggle = screen.getByRole("button", { name: "Mở danh sách hội thoại" });

        await user.click(toggle);
        await screen.findByRole("dialog");
        await user.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

        // Bung sidebar ra thì flyout phải tự đóng, không đè lên danh sách vừa
        // lộ ra ở cùng mép trái.
        await user.click(toggle);
        await screen.findByRole("dialog");
        await user.click(screen.getByRole("button", { name: "Mở rộng thanh điều hướng" }));
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
      }, 15000);
    });
  });

  it("mở rộng: ngôn ngữ và chủ đề xếp dọc, kèm nhãn chữ", async () => {
    renderSidebar();
    const language = await screen.findByRole("combobox", { name: /Ngôn ngữ/ });
    const theme = screen.getByRole("button", { name: /Giao diện/ });

    // Xếp dọc chứ không phải hàng ngang: sau khi thêm nhãn, hàng ngang bị `truncate`
    // cắt thành "Ngôn…"/"Giá…" — tệ hơn không có nhãn.
    expect(language.parentElement).toBe(theme.parentElement);
    expect(language.compareDocumentPosition(theme)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);

    // Có nhãn chữ: icon trơn không nói được đây là gì.
    expect(language).toHaveTextContent("Ngôn ngữ");
    expect(language).toHaveTextContent("Tiếng Việt");
    expect(theme).toHaveTextContent("Giao diện");
  }, 15000);

  // Rail thu gọn: nhãn chữ **không** hiện (80px không chứa nổi), chỉ còn icon —
  // và phải xếp dọc vì 32 + 8 + 36 = 76px > 56px khả dụng.
  it("thu gọn: ngôn ngữ và sáng/tối chỉ còn icon, xếp dọc", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

    const language = screen.getByRole("combobox", { name: /Ngôn ngữ/ });
    const theme = screen.getByRole("button", { name: "Chuyển sang giao diện sáng" });

    // Không chữ nhìn thấy; tên trợ năng vẫn đầy đủ (chỉ ở dạng thuộc tính).
    expect(language).not.toHaveTextContent("Tiếng Việt");
    expect(theme).not.toHaveTextContent("Giao diện");
    // `aria-label` không có tiền tố "Giao diện" ở rail, vì không có chữ đó hiện.
    expect(theme).toHaveAccessibleName("Chuyển sang giao diện sáng");

    // Xếp dọc: hai ô là **anh em** cùng cha, không phải hai phần tử lồng nhau.
    // `getBoundingClientRect` không dùng được ở đây — jsdom trả về 0 cho mọi
    // phép đo, nên so `top` sẽ cho `0 > 0` là sai.
    const row = language.closest("[data-slot='language-select']")?.parentElement;
    expect(row).toContainElement(language);
    expect(row).toContainElement(theme);
    expect(language).not.toContainElement(theme);
  }, 15000);

  it("thu gọn: nút mở rộng sidebar nằm giữa rail", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

    const expand = screen.getByRole("button", { name: "Mở rộng thanh điều hướng" });
    // `justify-between` sẽ đẩy nút sang mép trái khi không còn khối tiêu đề bên
    // cạnh; rail phải giữ cảm giác cột icon nên nút nằm giữa.
    expect(expand.parentElement).toHaveClass("justify-center");
  }, 15000);

  // Chân sidebar phải **ở đáy** ở cả hai trạng thái. Trước đây khi thu gọn,
  // `SessionList` không render nên footer trôi lên sát `nav` và treo lơ lửng giữa
  // màn hình. `mt-auto` trên footer + `flex-1` trên `nav` là cặp đẩy nhau.
  it("chân sidebar có mt-auto và nav có flex-1 để giữ chân ở đáy", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

    // Khối chân là **cha** của nút đăng xuất (không phải `lastElementChild`:
    // portal của Radix sinh thêm phần tử con ở cuối `<aside>`).
    const footer = screen.getByRole("button", { name: "Đăng xuất" }).parentElement;
    expect(footer).toHaveClass("mt-auto");
    // Nav phải hấp thụ khoảng trống, nếu không `mt-auto` của footer không có
    // gì để đẩy.
    const nav = screen.getByRole("navigation", { name: "Điều hướng chính" });
    expect(nav).toHaveClass("md:flex-1");
  }, 15000);

  // Bỏ padding khi thu gọn: cả ba nút thu về ô vuông `size-8` không padding,
  // thành cột icon thẳng hàng khớp với các mục điều hướng ngay trên.
  it("thu gọn: ba nút chân sidebar không padding", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

    // Nút ngôn ngữ có `role="combobox"` (Radix `SelectTrigger`), **không** phải
    // `button` — tìm sai role thì test fail với lý do không liên quan tới padding.
    const targets = [
      screen.getByRole("combobox", { name: /Ngôn ngữ/ }),
      screen.getByRole("button", { name: /Chuyển sang giao diện sáng/ }),
      screen.getByRole("button", { name: "Đăng xuất" }),
    ];
    for (const target of targets) {
      // `\bp[xy]-\d` bắt `p-2`/`px-3`/`py-2`, không bắt nhầm `rounded-lg` hay
      // `disabled:opacity-50` — `\b` chặn phần `p` trong `opacity`.
      expect(target.className, target.getAttribute("data-slot") ?? target.tagName).not.toMatch(/\bp[xy]-\d/);
      // Ô vuông 32px — khớp với các mục điều hướng ngay trên.
      expect(target.className).toMatch(/\bsize-8\b/);
    }
    // Canh giữa bằng `items-center` của cha flex, không cần `mx-auto` từng nút.
    expect(screen.getByRole("button", { name: "Đăng xuất" }).className).not.toContain("mx-auto");
  }, 15000);

  // Nút ngôn ngữ là `SelectTrigger`, có `border border-input` **trong component**;
  // hai nút kia dùng `variant="ghost"` nên không viền. Ở rail 80px ô này thành
  // nút duy nhất có viền — nhìn như ba nút khác nhau. Ở bản mở rộng thì viền đúng
  // vì nó đọc là một ô nhập.
  //
  // Dùng `border-transparent` chứ không `border-0`: `twMerge` xem `border-input`
  // và `border-0` là cùng nhóm rồi **xoá mất** `border-0`, còn `border-transparent`
  // thì thay đúng màu viền và `border-width` của component vẫn giữ nên kích thước
  // không đổi. Test này chặt cả hai: vừa phải có màu trong suốt, vừa phải bỏ
  // `border-0` để không ai vô tình viết lại bằng nó.
  it("thu gọn: nút ngôn ngữ trong suốt, mở rộng thì giữ viền", async () => {
    const user = renderSidebar();
    const trigger = () => screen.getByRole("combobox", { name: /Ngôn ngữ/ });

    expect(trigger().className, "mở rộng phải có viền").toContain("border-input");
    await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));
    expect(trigger().className, "thu gọn: viền phải trong suốt").toContain("border-transparent");
    // `border-0` bị `twMerge` xoá mất, nên class này vô nghĩa — chặn để khỏi
    // ai tưởng nó có tác dụng rồi viết lại.
    expect(trigger().className).not.toContain("border-0");
  }, 15000);

  it("mục điều hướng dùng icon vector thay cho ký tự Unicode", async () => {
    renderSidebar();
    const link = await screen.findByRole("link", { name: "Trò chuyện" });

    // Icon `<svg>` của lucide, không phải ký tự kiểu cũ (`◌`): ký tự Unicode đổi
    // hình dáng theo font của hệ điều hành, còn SVG vẽ bằng `currentColor`.
    const icon = link.querySelector("svg");
    expect(icon).not.toBeNull();
    expect(icon).toHaveAttribute("aria-hidden", "true");
    // Nhãn chữ vẫn là văn bản thật bên cạnh icon, không nằm trong `<title>`.
    expect(link.querySelector("span")).toHaveTextContent("Trò chuyện");
  }, 15000);

  // Hồi quy: `<TooltipTrigger asChild>` **clone** con rồi đè `className` của
  // trigger lên nó. Vì `NavItem` thu gọn nằm trong tooltip, `className` dạng hàm
  // của `NavLink` từng biến thành **chuỗi function chưa từng chạy** — mọi padding
  // và `size-9` mất sạch mà không có lỗi console nào.
  it("class của mục điều hướng được áp dụng thật khi thu gọn", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

    const link = screen.getByRole("link", { name: "Trò chuyện" });
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
    await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

    const link = screen.getByRole("link", { name: "Trò chuyện" });
    const matches = link.getAttribute("class")?.match(/size-9/g) ?? [];
    expect(matches).toHaveLength(1);
  }, 15000);

  // Mục đang active vẫn phải nổi bật sau khi bỏ `className` dạng hàm: `isActive`
  // giờ do `useMatch` tính, nên phải chốt lại để không mất dấu hiệu này vô hình.
  it("mục đang active vẫn được đánh dấu", async () => {
    const user = renderSidebar();
    await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));

    // `renderSidebar()` mở route `/sessions/4`, nên **không** có mục nào active
    // với `end` — mọi mục phải mang class của trạng thái không active. Điều này
    // bắt được lỗi ngược lại: `isActive` luôn `true` sẽ làm mọi mục nổi bật.
    const chat = screen.getByRole("link", { name: "Trò chuyện" });
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

    await user.click(await screen.findByRole("button", { name: "Thu gọn thanh điều hướng" }));
    expect(screen.getByRole("link", { name: "Trạng thái" }).className).toContain("bg-accent");
    expect(screen.getByRole("link", { name: "Trung tâm" }).className).not.toContain("bg-accent");
  }, 15000);
});
