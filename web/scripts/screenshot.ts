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

const now2 = Date.now();
/**
 * Hội thoại mẫu. Cần có dữ liệu thật để ảnh phê bình đúng **mật độ chữ** — danh
 * sách rỗng thì không phát hiện được lỗi cắt chữ hay ô tìm kiếm chật.
 */
const SESSIONS = [
  ["Ke hoach lam viec tuan nay", 2 * 60_000],
  ["Fix loi dong bo SQLite", 95 * 60_000],
  ["Bao cao doanh thu Q3", 6 * 3_600_000],
  ["Hoi thao MCP server", 2 * 86_400_000],
  ["Draft bai blog ky thuat", 20 * 86_400_000],
].map(([title, age], index) => ({
  id: 10 + index,
  channel: "web",
  chat_id: "web:admin",
  user_id: "web:admin",
  title: String(title),
  archived: false,
  created_at: new Date(now2 - Number(age)).toISOString(),
  updated_at: new Date(now2 - Number(age)).toISOString(),
}));

/**
 * Trả đúng payload mà API thật sẽ trả cho từng path.
 *
 * **Mọi route mới phải thêm vào đây.** Trả `{}` cho một route có kiểu danh
 * sách khiến `query.data.<mảng>.length` ném `undefined.length` giữa lúc render —
 * React gỡ cả cây và trang trắng, nhưng lỗi **chỉ hiện trong console**, nên
 * `pnpm shot` chết vì timeout chờ selector chứ không báo ra nguyên nhân.
 * Đó là lý do `tools`/`mcp`/`usage` phải ở đây chứ không "chỉ cần để trống".
 */
function mockFor(pathname: string): unknown {
  if (pathname === "/api/auth/me") return { user_id: "web:admin" };
  if (pathname === "/api/agents") return { agents: AGENTS, viewer_role: "admin" };
  if (pathname === "/api/status") return STATUS;
  if (pathname === "/api/audit") return AUDIT;
  if (pathname === "/api/sessions") return { sessions: SESSIONS };
  if (pathname === "/api/tools") {
    return { tools: [], total: 0, viewer_role: "admin", rbac_enabled: false };
  }
  if (pathname === "/api/mcp") return { servers: [], total: 0 };
  if (pathname === "/api/usage") return { days: [], total_tokens: 0 };
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

      // Đặt chủ đề đúng cơ chế của app: `:root` là TỐI, chủ đề sáng bật bằng
      // class `.light` trên `<html>` (xem `src/index.css` và `src/lib/theme.tsx`).
      // Trước đây script chỉ toggle `.dark`, hợp với lúc mọi token tối nằm sau
      // class `.dark`; với cơ chế mới, bỏ `.dark` mà không thêm `.light` thì vẫn
      // ra nền tối — hai ảnh "light" và "dark" trùng nhau y hệt mà không có lỗi
      // nào được báo ra. Ghi kèm `localStorage` để khớp đường lúc ứng dụng
      // tự khởi tạo.
      const applyTheme = () => {
        const root = document.documentElement;
        if (!root) return;
        root.classList.remove("dark", "light");
        root.classList.add(darkMode ? "dark" : "light");
        root.style.colorScheme = darkMode ? "dark" : "light";
        try {
          window.localStorage.setItem("bean.theme", darkMode ? "dark" : "light");
        } catch {
          // storage bị chặn thì bỏ qua: class trên `<html>` đã đủ để CSS áp dụng.
        }
      };
      if (document.documentElement) applyTheme();
      else new MutationObserver(applyTheme).observe(document, { childList: true, subtree: true });
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

/**
 * Chụp riêng sidebar: đây là phần sửa gần nhất (rail thu gọn, icon menu, hàng
 * ngôn ngữ + sáng/tối), nên cần nhìn **thay đổi bề mặt** chứ không chỉ tin test
 * DOM xanh.
 *
 * Chụp thẳng vào Vite dev server (`:5173`) để ảnh phản ánh đúng những gì người
 * dùng đang xem khi làm việc, không phải bản `dist/` đã đóng gói.
 */
const DEV_URL = process.env.SHOT_URL ?? "http://localhost:5173";

async function shotSidebar(
  name: string,
  width: number,
  dark: boolean,
  collapsed: boolean,
  openHistory = false,
  path = "/chat",
  clipFooter = false,
) {
  const page = await browser.newPage();
  page.on("pageerror", (e) => console.log(`  [${name}] PAGEERROR ${e.message}`));
  await page.setViewport({ width, height: 900, deviceScaleFactor: 2 });
  await page.setRequestInterception(true);
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith("/api/")) {
      // Dùng CHUNG `mockFor` với `shot()`: payload phải khớp đúng hợp đồng
      // `ts-rs`. Trả `{}` rỗng như thử đầu làm `ActivityFeed` đọc
      // `.length` của `undefined` và React gỡ cả cây — trang trắng mà không có
      // lỗi nào được báo ra ngoài console.
      request.respond({
        contentType: "application/json",
        body: JSON.stringify(mockFor(url.pathname)),
      });
      return;
    }
    request.continue();
  });
  await page.evaluateOnNewDocument(
    (confirms, darkMode, isCollapsed) => {
      // Chỉ **đường `/api/ws`** mới bị giả lập, mọi WebSocket khác (đáng chú ý là
      // của `@vite/client`) vẫn là socket thật.
      //
      // Vì sao không thay thẳng `window.WebSocket` như `shot()` bên trên:
      // ở Vite dev, `@vite/client` dựng WebSocket ngay ở document-start, nên
      // thay toàn cục làm hỏng HMR **và** React refresh — biểu hiện là
      // `Cannot read properties of null (reading 'useRef')` và trang trắng.
      // `shot()` chạy static server nên không có đối thủ này mới làm được.
      //
      // Cần giả lập vì `HubPage` đọc `pending_confirms.length` ngay ở render
      // đầu; thiếu `Sync` thì `undefined.length` làm React **gỡ cả cây** — lỗi
      // chỉ hiện trong console, trang trắng trông như app chết hẳn.
      const RealWebSocket = window.WebSocket;
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
      (window as unknown as { WebSocket: unknown }).WebSocket = class extends FakeSocket {
        constructor(url: string | URL, protocols?: string | string[]) {
          super();
          if (!String(url).includes("/api/ws")) {
            // Không phải đường của Bean → trả về socket thật.
            return new RealWebSocket(url, protocols) as never;
          }
        }
      };
      const apply = () => {
        const root = document.documentElement;
        if (!root) return;
        root.classList.remove("dark", "light");
        root.classList.add(darkMode ? "dark" : "light");
        root.style.colorScheme = darkMode ? "dark" : "light";
        // Đặt sẵn lựa chọn thu gọn để lần render đầu đã đúng bề rộng — nếu
        // không, ảnh sẽ chụp trạng thái mặc định rồi mới nhảy, đúng thứ cần
        // tránh khi dùng ảnh để phê bình bố cục.
        try {
          window.localStorage.setItem("bean.theme", darkMode ? "dark" : "light");
          window.localStorage.setItem("bean.sidebar.collapsed", isCollapsed ? "1" : "0");
        } catch { /* storage bị chặn: bỏ qua */ }
      };
      if (document.documentElement) apply();
      else new MutationObserver(apply).observe(document, { childList: true, subtree: true });
    },
    CONFIRMS,
    dark,
    collapsed,
  );
  // `domcontentloaded` + chờ selector, **không** dùng `networkidle0`: `ws.ts`
  // giữ WebSocket `/api/ws` mở suốt phiên nên "network idle" không bao giờ
  // tới — `goto` sẽ treo tới hết timeout và báo sai nguyên nhân.
  // `new URL` thay vì nối chuỗi: `DEV_URL` có dấu `/` cuối thì ghép tay ra
  // `http://host//chat`, mà `//` ở đầu *đường dẫn* lại bị coi là authority —
  // trình duyệt quay về `/` và ảnh chụp nhầm trang HUB (nơi cố ý không có danh
  // sách hội thoại, nên nút lịch sử biến mất và báo lỗi rất khó đoán).
  await page.goto(new URL(path, DEV_URL).toString(), { waitUntil: "domcontentloaded" });
  await page.waitForSelector('[data-testid="sidebar-shell"]', { timeout: 10_000 });
  // Mở flyout bằng ** cú bấm thật** chứ không ép state trong React: đây là cách
  // duy nhất xác minh cả đường bấm lẫn hiệu ứng focus của Radix hoạt động.
  if (openHistory) {
    await page.click('[data-slot="history-toggle"]');
    await page.waitForSelector('[data-slot="history-flyout"]', { timeout: 5_000 });
  }
  await new Promise((r) => setTimeout(r, 400));
  const file = resolve(outDir, `${name}.png`);
  if (clipFooter) {
    // Chân sidebar bị danh sách hội thoại đẩy xuống dưới màn hình, nên ảnh toàn
    // trang không soi được phần cần xem. Đo **chính cái khối chân** thay vì suy từ
    // ô tìm kiếm: lần trước cắt từ dưới ô tìm kiếm thì vẫn lọt nguyên khối danh
    // sách và không thấy nhãn — mốc phải là phần tử cần soi, không phải thứ nằm
    // ngay trên nó.
    const box = await page.$eval('[data-testid="sidebar-shell"]', (shell) => {
      // Khối cuối cùng trong `<aside>` là chân sidebar (ngôn ngữ/chủ đề/đăng xuất).
      const footer = shell.lastElementChild;
      const rect = (footer ?? shell).getBoundingClientRect();
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
    });
    await page.screenshot({ path: file, clip: box });
  } else {
    await page.screenshot({ path: file, fullPage: false });
  }
  console.log(`✓ ${file}`);
  await page.close();
}


try {
  // Ảnh sidebar đi qua `/chat` **không phải `/`**: trang HUB cố tình không có
  // danh sách hội thoại (`showSessions = pathname !== "/"`), nên chụp ở HUB sẽ
  // bỏ sót đúng phần cần kiểm tra.
  await shotSidebar("sidebar-expanded-dark", 1440, true, false, false, "/chat");
  await shotSidebar("sidebar-collapsed-dark", 1440, true, true, false, "/chat");
  await shotSidebar("sidebar-flyout-dark", 1440, true, true, true, "/chat");
  await shotSidebar("sidebar-flyout-light", 1440, false, true, true, "/chat");
  // Chân sidebar: ảnh toàn trang bị danh sách hội thoại đẩy ra dưới màn hình,
  // nên phần cần soi không bao giờ lọt vào khung. `clipFooter` cắt đúng vùng đó
  // thay vì viết thêm một hàm chụp — hàm riêng thì phải dựng lại toàn bộ đường đi
  // (page mới, `evaluateOnNewDocument`, fake WebSocket…) và dễ lệch với
  // `shotSidebar` sau mỗi lần sửa.
  // Chân sidebar: ảnh toàn trang bị danh sách hội thoại đẩy ra dưới màn hình,
  // nên phần cần soi không bao giờ lọt vào khung. `clipFooter` cắt đúng vùng đó
  // thay vì viết thêm một hàm chụp — hàm riêng thì phải dựng lại toàn bộ đường đi
  // (page mới, `evaluateOnNewDocument`, fake WebSocket…) và dễ lệch với
  // `shotSidebar` sau mỗi lần sửa.
  await shotSidebar("footer-expanded-dark", 1440, true, false, false, "/chat", true);
  await shotSidebar("footer-expanded-light", 1440, false, false, false, "/chat", true);
  await shotSidebar("footer-collapsed-dark", 1440, true, true, false, "/chat", true);
  await shot("hub-desktop-light", 1440, 900, false);
  await shot("hub-desktop-dark", 1440, 900, true);
  await shot("hub-mobile-light", 390, 844, false);
  await shot("hub-mobile-dark", 390, 844, true);
} finally {
  preview.kill();
  await browser.close();
}
