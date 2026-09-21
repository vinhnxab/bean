import "@fontsource-variable/inter";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "@/App";
import "@/index.css";
import { I18nProvider } from "@/i18n";

const container = document.getElementById("root");

if (container) {
  createRoot(container).render(
    <StrictMode>
      <I18nProvider>
        <App />
      </I18nProvider>
    </StrictMode>,
  );
} else {
  // Không ném lỗi (mục 0.8: không panic trong code sản phẩm) — chỉ báo rõ để dễ sửa index.html.
  console.error("BeanAgent: không tìm thấy phần tử #root trong index.html");
}
