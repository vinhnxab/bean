import { mkdirSync } from "node:fs";
import { spawn } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import puppeteer from "puppeteer-core";

/**
 * Chụp screenshot HUB để **tự phê bình** trước khi báo xong.
 *
 * Không dùng ảnh chụp giả: script chạy app thật (`dist/`), chặn `/api/*` bằng dữ
 * liệu đúng hợp đồng `ts-rs`, rồi chụp bằng Chrome thật. Nhờ vậy ảnh phản ánh đúng
 * những gì người dùng thấy — kể cả lỗi bố cục mà test DOM không bắt được.
 *
 * Chạy: `pnpm build && pnpm shot`.
 */
const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const outDir = resolve(root, "../.screenshots");
const PORT = Number(process.env.SHOT_PORT ?? 4180);
mkdirSync(outDir, { recursive: true });

/** Agent mô phỏng — phản ánh `[[roles]]` trong `bean.example.toml`. */
const AGENTS = [
  { role: "developer", status: "working", summary: "đang thực hiện lượt", risks: [], relation: "manages" },
  { role: "monitor", status: "idle", summary: "không có việc nào đang chạy", risks: [], relation: "manages" },
  {
    role: "marketing",
    status: "awaiting_you",
    summary: "1 hành động đang chờ bạn duyệt",
    risks: ["marketing_publish · đăng bài Q3 lên blog"],
    relation: "manages",
  },
  { role: "qa", status: "working", summary: "đang thực hiện lượt", risks: [], relation: "reviews" },
  {
    role: "finance-readonly",
    status: "idle",
    summary: "không có việc nào đang chạy",
    risks: [],
    relation: "manages",
  },
  { role: "security-scan", status: "idle", summary: "không có việc nào đang chạy", risks: [], relation: "alerts_directly" },
];

const now = Date.now();
const AUDIT = {
  entries: [
    { ts: new Date(now - 60_000).toISOString(), session: 7, channel: "web", tool: "run_shell", args: {}, ok: true, decision: "allowed", decided_by: "web:admin", error: null },
    { ts: new Date(now - 240_000).toISOString(), session: 7, channel: "web", tool: "write_file", args: {}, ok: true, decision: "allowed", decided_by: "web:admin", error: null },
    { ts: new Date(now - 900_000).toISOString(), session: 7, channel: "web", tool: "marketing_publish", args: {}, ok: false, decision: "denied", decided_by: "web:admin", error: null },
  ],
};

const STATUS = {
  version: "0.1.0",
  model: "claude-sonnet-5",
  max_steps: 25,
  daily_token_budget: 2_000_000,
  tokens_used: 184_320,
  uptime_seconds: 248_400,
  channels: ["web", "telegram"],
};

/** Hàng chờ duyệt, đúng cấu trúc `PendingConfirm` đã sinh binding. */
const CONFIRMS = [
  {
    confirm_id: "c1",
    session_id: 7,
    run_id: "run-a",
    prompt: 'run_shell --command "nmap -sS -p- 10.0.3.0/24"',
    risk: "dangerous",
    allow_session_option: false,
    timeout_seconds: 240,
    role: "security-scan",
  },
  {
    confirm_id: "c2",
    session_id: 7,
    run_id: "run-b",
    prompt: 'marketing_publish --title "Kết quả Q3"',
    risk: "confirm",
    allow_session_option: true,
    timeout_seconds: 180,
    role: "marketing",
  },
];

/** Trả đúng payload mà API thật sẽ trả cho từng path. */
function mockFor(pathname: string): unknown {
  if (pathname === "/api/auth/me") return { user_id: "web:admin" };
  if (pathname === "/api/agents") return { agents: AGENTS, viewer_role: "admin" };
  if (pathname === "/api/status") return STATUS;
  if (pathname === "/api/audit") return AUDIT;
  if (pathname === "/api/sessions") return { sessions: [] };
  return {};
}

const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_PATH ?? "/usr/bin/google-chrome",
  args: ["--no-sandbox", "--disable-dev-shm-usage", "--font-render-hinting=none"],
});

async function shot(name: string, width: number, height: number, dark: boolean) {
  const page = await browser.newPage();
  page.on("console", (m) => console.log(`  [${name}] ${m.text()}`));
  page.on("pageerror", (e) => console.log(`  [${name}] PAGEERROR ${e.message}`));
  await page.setViewport({ width, height, deviceScaleFactor: 2 });

  await page.setRequestInterception(true);
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith("/api/")) {
      request.respond({
        contentType: "application/json",
        body: JSON.stringify(mockFor(url.pathname)),
      });
      return;
    }
    request.continue();
  });

  // Bơm `Sync` để hàng chờ duyệt có dữ liệu thật; bật dark mode trước khi React
  // render để ảnh không nháy trắng.
  //
  // KHÔNG mở rộng `WebSocket` thật: static server không hỗ trợ nâng cấp nên kết
  // nối thất bại ngay, `onerror` → `close()` → `stop()` xoá `onmessage`, và tin
  // nhắn bơm vào bị rơi. Thay bằng đối tượng duck-typed có sẵn `onmessage`.
  await page.evaluateOnNewDocument(
    (confirms, darkMode) => {
      // PHẢI cài WebSocket trước, rồi mới bật dark: ở thời điểm
      // document-start, `document.documentElement` là `null`, nên dòng dark bên
      // dưới sẽ ném TypeError và làm mọi thứ phía sau — kể cả việc thay WebSocket —
      // không bao giờ chạy. Đó là lý do hàng chờ duyệt biến mất trong ảnh mà
      // không có lỗi nào được báo ra.
      const payload = JSON.stringify({
        type: "sync",
        running: [{ session_id: 7, run_id: "run-a", role: "developer" }],
        pending_confirms: confirms,
      });
      class FakeSocket {
        readyState = 1;
        onopen: (() => void) | null = null;
        onmessage: ((event: MessageEvent) => void) | null = null;
        onerror: (() => void) | null = null;
        onclose: (() => void) | null = null;
        send() {}
        close() {}
        constructor() {
          setTimeout(() => this.onopen?.(), 10);
          // Gửi lặp lại để không phụ thuộc thứ tự gán handler.
          let tries = 0;
          const timer = setInterval(() => {
            this.onmessage?.({ data: payload } as MessageEvent);
            if (++tries >= 10) clearInterval(timer);
          }, 120);
        }
      }
      (window as unknown as { WebSocket: unknown }).WebSocket = FakeSocket;

      // Bật dark ngay khi `<html>` có mặt; nếu chưa thì chờ tới khi có.
      const applyDark = () =>
        document.documentElement?.classList.toggle("dark", darkMode);
      if (document.documentElement) applyDark();
      else new MutationObserver(applyDark).observe(document, { childList: true, subtree: true });
    },
    CONFIRMS,
    dark,
  );

  await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle0" });
  // Chờ thẳng hàng chờ duyệt xuất hiện thay vì đoán thời gian: nó đến từ WebSocket
  // nên không nằm trong "network idle", và chụp sớm sẽ ra ảnh thiếu hàng trăm
  // ký tự cảm giác về sản phẩm.
  await page
    .waitForSelector('[data-testid="hub-confirm-queue"]', { timeout: 10_000 })
    .catch(() => console.warn(`⚠ ${name}: không thấy hàng chờ duyệt`));
  await new Promise((r) => setTimeout(r, 400));
  const file = resolve(outDir, `${name}.png`);
  await page.screenshot({ path: file, fullPage: true });
  console.log(`✓ ${file}`);
  await page.close();
}

// Dùng static server của Python thay vì `vite preview`: không phụ thuộc port đã
// bị chiếm bởi process cũ, và `dist/` là thứ thật sự được phục vụ.
const preview = spawn("python3", ["-m", "http.server", String(PORT), "--bind", "127.0.0.1", "--directory", resolve(root, "dist")], {
  stdio: "ignore",
});

/** Chờ server sẵn sàng thay vì đoán bằng `sleep` — chạy trên máy chậm sẽ hỏng. */
async function waitForServer(url: string, timeoutMs: number) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // chưa lên — thử lại
    }
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error(`static server không sẵn sàng sau ${timeoutMs}ms: ${url}`);
}

await waitForServer(`http://127.0.0.1:${PORT}/`, 20_000);

try {
  await shot("hub-desktop-light", 1440, 900, false);
  await shot("hub-desktop-dark", 1440, 900, true);
  await shot("hub-mobile-light", 390, 844, false);
  await shot("hub-mobile-dark", 390, 844, true);
} finally {
  preview.kill();
  await browser.close();
}
