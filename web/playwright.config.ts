import { defineConfig } from "@playwright/test";

/**
 * Playwright E2E UI (mục 3, M16) — chạy browser thật against `bean serve --fake-llm`.
 *
 * # Vì sao một server dùng chung và `workers: 1`
 *
 * FakeProvider tiêu thụ kịch bản **tuần tự toàn cục** (cursor AtomicUsize), nên
 * thứ tự message gửi lên phải khớp thứ tự response trong `e2e-ui/fake-script.json`.
 * Chạy tuần tự một file là cách giữ bất biến đó mà không cần khoá ngoài.
 *
 * # Vì sao `channel: "chrome"`
 *
 * Máy chạy agent đã có Chrome thật (M26 CDP dùng nó) — dùng luôn để không phải
 * tải browser của Playwright. Nếu máy không có Chrome, cài bằng `pnpm exec
 * playwright install chromium` rồi đổi `channel` thành `"chromium"`.
 */
export default defineConfig({
  testDir: "./e2e-ui",
  timeout: 90_000,
  expect: { timeout: 15_000 },
  workers: 1,
  fullyParallel: false,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: process.env.BEAN_E2E_BASE_URL ?? "http://127.0.0.1:18944",
    channel: "chrome",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "bash e2e-ui/serve.sh",
    // `/login` là file tĩnh qua SPA fallback — không cần auth, đủ làm tín hiệu "server sống".
    url: "http://127.0.0.1:18944/login",
    reuseExistingServer: false,
    timeout: 240_000,
    // Mặc định Playwright chém SIGKILL — serve.sh không kịp dọn temp. SIGTERM cho
    // trap trong serve.sh chạy (`cleanup` kill bean rồi xoá thư mục temp).
    gracefulShutdown: { signal: "SIGTERM", timeout: 10_000 },
  },
});
