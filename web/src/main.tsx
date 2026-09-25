import "@fontsource-variable/inter";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "@/App";
import "@/index.css";
import { I18nProvider } from "@/i18n";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, refetchOnWindowFocus: false },
  },
});

const container = document.getElementById("root");

if (container) {
  createRoot(container).render(
    <StrictMode>
      <I18nProvider>
        <QueryClientProvider client={queryClient}>
          <App />
        </QueryClientProvider>
      </I18nProvider>
    </StrictMode>,
  );
} else {
  // Không ném lỗi (mục 0.8: không panic trong code sản phẩm) — chỉ báo rõ để dễ sửa index.html.
  console.error("BeanAgent: không tìm thấy phần tử #root trong index.html");
}
