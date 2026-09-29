import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import type { ReactElement } from "react";
import { MemoryRouter } from "react-router";

import { I18nProvider } from "@/i18n";
import { ThemeProvider } from "@/lib/theme";

/**
 * Bọc đúng các provider mà app thật dùng, theo cùng thứ tự với `main.tsx`.
 * Thiếu `ThemeProvider` ở đây thì mọi màn hình có nút đổi chủ đề sẽ ném lỗi
 * ngay khi test — đó là điều tốt, nhưng chỉ tốt nếu helper này luôn khớp app.
 */
export function renderManagement(ui: ReactElement, initialEntries = ["/"]) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <I18nProvider>
          <MemoryRouter initialEntries={initialEntries}>{ui}</MemoryRouter>
        </I18nProvider>
      </ThemeProvider>
    </QueryClientProvider>,
  );
}
