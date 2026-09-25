import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import type { ReactElement } from "react";
import { MemoryRouter } from "react-router";

import { I18nProvider } from "@/i18n";

export function renderManagement(ui: ReactElement, initialEntries = ["/"]) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <I18nProvider>
        <MemoryRouter initialEntries={initialEntries}>{ui}</MemoryRouter>
      </I18nProvider>
    </QueryClientProvider>,
  );
}
