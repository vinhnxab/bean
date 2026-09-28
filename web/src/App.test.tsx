import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import App from "@/App";
import { I18nProvider } from "@/i18n";
import { testServer } from "@/test/server";

function renderApp() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <I18nProvider>
        <App />
      </I18nProvider>
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
});
