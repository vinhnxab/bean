import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import App from "@/App";
import { I18nProvider } from "@/i18n";
import { ThemeProvider } from "@/lib/theme";
import { testServer } from "@/test/server";

function renderApp() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <I18nProvider>
          <App />
        </I18nProvider>
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

describe("App", () => {
  it("chuyển về trang đăng nhập khi API trả 401", async () => {
    window.history.pushState({}, "", "/");
    testServer.use(
      http.get("/api/auth/me", () =>
        HttpResponse.json({ code: "unauthorized", message: "no" }, { status: 401 }),
      ),
    );

    renderApp();

    await waitFor(() => expect(screen.getByRole("heading", { name: /Đăng nhập Bean/ })).toBeInTheDocument());
    expect(screen.getByLabelText("Mật khẩu")).toBeInTheDocument();
  });

  // Route lazy: `App.tsx` nạp từng màn bằng `React.lazy`. Các test này chốt đúng một
  // thứ — màn **vẫn mở được sau khi tách chunk**. Nếu import đường dẫn sai, hoặc
  // `Suspense` đặt sai chỗ khiến route không bao giờ render, test đỏ.
  //
  // Vì sao chờ 3 s chứ không dùng mặc định 1 s: các test này cùng một file, mỗi
  // test lại kích hoạt một `import()` khác nhau của Vite. Chạy cả file thì các
  // lần import đó xếp hàng, `findByRole` hết giờ chờ mặc định **trước khi** chunk
  // kịp tới — test fail một cách ngẫu nhiên, chạy đơn lẻ thì xanh. Chờ lâu hơn
  // là sửa đúng chỗ, không phải làm yếu điều kiện khẳng định.
  //
  // **Mọi** test mở route lazy đều phải dùng hằng này. Nó khai báo TRƯỚC các
  // test dùng tới (biến `const` có TDZ) — trước đây chỉ 3 test dưới có, còn hai
  // test `/tools` và `/mcp` dùng mặc định nên rơi ngẫu nhiên khi chạy song song
  // với 22 file khác: xanh khi `pnpm vitest run src/App.test.tsx`, đỏ trong
  // `make check`.
  const lazyTimeout = { timeout: 3000 };

  // Hai màn mới phải thật sự nằm trong router — thiếu `<Route>` thì link trong
  // sidebar vẫn hiện ra nhưng bấm vào lại rơi về `/`, rất khó thấy nếu chỉ nhìn UI.
  it("đường dẫn /tools mở màn Tools", async () => {
    window.history.pushState({}, "", "/tools");
    renderApp();
    expect(await screen.findByRole("heading", { name: "Tools" }, lazyTimeout)).toBeInTheDocument();
  });

  it("đường dẫn /mcp mở màn MCP", async () => {
    window.history.pushState({}, "", "/mcp");
    renderApp();
    expect(await screen.findByRole("heading", { name: "MCP server" }, lazyTimeout)).toBeInTheDocument();
  });

  it("đường dẫn /memory mở màn Bộ nhớ", async () => {
    window.history.pushState({}, "", "/memory");
    renderApp();
    expect(await screen.findByRole("heading", { name: /Bộ nhớ/ }, lazyTimeout)).toBeInTheDocument();
  });

  it("đường dẫn /status mở màn Trạng thái", async () => {
    window.history.pushState({}, "", "/status");
    renderApp();
    // Khớp **tiêu đề trang** (`status.title`), không phải nhãn nav: `nav.status`
    // là "Trạng thái" còn tiêu đề là "Trạng thái hệ thống" — đây là hai chỗ
    // khác nhau có chủ đích, test phải trỏ đúng chỗ.
    expect(
      await screen.findByRole("heading", { name: /Trạng thái hệ thống/ }, lazyTimeout),
    ).toBeInTheDocument();
  });

  it("đường dẫn /audit mở màn Nhật ký", async () => {
    window.history.pushState({}, "", "/audit");
    renderApp();
    expect(await screen.findByRole("heading", { name: "Audit" }, lazyTimeout)).toBeInTheDocument();
  });
});
