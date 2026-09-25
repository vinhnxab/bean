// Thiết lập cho mọi test của Vitest (agents.md mục 3.2, 20).
// - jest-dom: matcher như `toBeInTheDocument()`, `toHaveTextContent()`.
// - KHÔNG bật `dangerouslySetInnerHTML` hay tài nguyên ngoài trong test (mục 0.9).
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterAll, afterEach, beforeAll } from "vitest";

import { testServer } from "@/test/server";

beforeAll(() => testServer.listen({ onUnhandledRequest: "error" }));
afterEach(() => {
  cleanup();
  testServer.resetHandlers();
});
afterAll(() => testServer.close());
