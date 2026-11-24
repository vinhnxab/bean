import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * `public/sw.js` là file tĩnh cho trình duyệt nhưng giữ bất biến quan trọng
 * nhất của PWA Bean: **network-first** và **không chạm `/api`**. Cache-first
 * với tên asset không hash sẽ tái diễn lỗi "bản cũ mãi" (mục 22.27) — test
 * này khoá chiến lược lại. Cách test: nạp module với `self`/`caches`/`fetch`
 * giả rồi bắn sự kiện như trình duyệt làm.
 */

type EventHandler = (event: unknown) => void;

/** Sự kiện đã đăng ký bởi sw.js, gom theo tên để test bắn lại. */
const handlers: Record<string, EventHandler[]> = {};

const fakeSelf = {
  location: { origin: "http://bean.test" },
  addEventListener: (name: string, handler: EventHandler) => {
    const list = handlers[name] ?? [];
    list.push(handler);
    handlers[name] = list;
  },
  skipWaiting: vi.fn(async () => undefined),
  clients: {
    claim: vi.fn(async () => []),
    matchAll: vi.fn(async (): Promise<Array<{ focus: () => Promise<void> }>> => []),
    openWindow: vi.fn(async () => undefined),
  },
  // Kiểu tham số tường minh để `mock.calls[0]` đọc được [title, options] trong test.
  registration: {
    showNotification: vi.fn(
      async (_title: string, _options?: { body?: string; data?: { url: string } }) => undefined,
    ),
  },
};

/** PushEvent tối thiểu: `data.text()` trả payload, `waitUntil` gom promise. */
function pushEvent(payload: string | null) {
  const { event, done } = lifecycleEvent();
  return {
    event: { ...event, data: payload === null ? null : { text: () => payload } },
    done,
  };
}

/** NotificationEvent tối thiểu: notification có `close` và `data`. */
function notificationEvent(data: { url?: string } | null) {
  const { event, done } = lifecycleEvent();
  return {
    event: { ...event, notification: { close: vi.fn(), data } },
    done,
  };
}

/** CacheStorage giả: `open` trả Cache trên Map, `keys`/`delete` theo tên cache. */
function makeFakeCaches() {
  const cachesMap = new Map<string, Map<string, Response>>();
  const api = {
    async open(name: string) {
      const store = cachesMap.get(name) ?? new Map<string, Response>();
      cachesMap.set(name, store);
      return {
        async match(key: Request | string): Promise<Response | undefined> {
          return store.get(typeof key === "string" ? key : key.url);
        },
        async put(key: Request | string, response: Response): Promise<void> {
          store.set(typeof key === "string" ? key : key.url, response);
        },
        async add(key: string): Promise<void> {
          store.set(key, await fetch(key));
        },
      };
    },
    async match(key: Request | string): Promise<Response | undefined> {
      // Giống CacheStorage.match thật: dò từng cache, trả hit đầu tiên.
      const url = typeof key === "string" ? key : key.url;
      for (const store of cachesMap.values()) {
        const hit = store.get(url);
        if (hit) return hit;
      }
      return undefined;
    },
    keys: async () => [...cachesMap.keys()],
    delete: async (name: string) => {
      cachesMap.delete(name);
    },
  };
  return { cachesMap, api };
}

function fire(name: string, event: unknown): void {
  for (const handler of handlers[name] ?? []) handler(event);
}

/** `waitUntil` gom promise để test chờ xong hẳn mới assert. */
function lifecycleEvent() {
  const promises: Promise<unknown>[] = [];
  return {
    event: { waitUntil: (p: Promise<unknown>) => promises.push(p) },
    done: () => Promise.all(promises),
  };
}

/** FetchEvent tối thiểu — sw.js chỉ dùng `request` và `respondWith`. */
function fetchEvent(request: Request) {
  const calls: { respondedWith?: Promise<Response> } = {};
  return {
    request,
    respondWith: (promise: Promise<Response>) => {
      calls.respondedWith = promise;
    },
    calls,
  };
}

/** Request `mode: navigate` — jsdom/undici không nhận mode này qua init. */
function navigateRequest(url: string): Request {
  const request = new Request(url);
  Object.defineProperty(request, "mode", { value: "navigate" });
  return request;
}

describe("service worker (public/sw.js)", () => {
  let cachesMock: ReturnType<typeof makeFakeCaches>;
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    vi.resetModules();
    for (const key of Object.keys(handlers)) delete handlers[key];
    fakeSelf.skipWaiting.mockClear();
    fakeSelf.clients.claim.mockClear();
    fakeSelf.clients.matchAll.mockClear();
    fakeSelf.clients.openWindow.mockClear();
    fakeSelf.registration.showNotification.mockClear();
    cachesMock = makeFakeCaches();
    fetchMock = vi.fn();
    vi.stubGlobal("self", fakeSelf);
    vi.stubGlobal("caches", cachesMock.api);
    vi.stubGlobal("fetch", fetchMock);
    // sw.js là file tĩnh cho trình duyệt (public/) — Vite cấm import từ public
    // vào module graph, nên đọc text rồi thực thi trong scope global đã stub.
    // sw.js không có import/export nên chạy được qua `new Function`.
    const source = await readFile(resolve(process.cwd(), "public/sw.js"), "utf8");
    new Function(source)();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("install: precache shell index.html rồi skipWaiting", async () => {
    fetchMock.mockResolvedValue(new Response("shell", { status: 200 }));
    const { event, done } = lifecycleEvent();

    fire("install", event);
    await done();

    expect(cachesMock.cachesMap.get("bean-ui-v1")?.get("/index.html")).toBeInstanceOf(Response);
    expect(fakeSelf.skipWaiting).toHaveBeenCalledOnce();
  });

  it("activate: xoá cache cũ, giữ cache hiện tại rồi claim clients", async () => {
    cachesMock.cachesMap.set("bean-ui-v0", new Map());
    cachesMock.cachesMap.set("bean-ui-v1", new Map());
    const { event, done } = lifecycleEvent();

    fire("activate", event);
    await done();

    expect(cachesMock.cachesMap.has("bean-ui-v0")).toBe(false);
    expect(cachesMock.cachesMap.has("bean-ui-v1")).toBe(true);
    expect(fakeSelf.clients.claim).toHaveBeenCalledOnce();
  });

  it("fetch: online thì trả response mạng và ghi vào cache cho lần offline", async () => {
    const response = new Response("fresh", { status: 200 });
    fetchMock.mockResolvedValue(response);
    const event = fetchEvent(new Request("http://bean.test/assets/index.js"));

    fire("fetch", event);

    const served = await event.calls.respondedWith;
    expect(await served?.text()).toBe("fresh");
    expect(cachesMock.cachesMap.get("bean-ui-v1")?.get("http://bean.test/assets/index.js")).toBeInstanceOf(
      Response,
    );
  });

  it("fetch: response không ok (404…) thì không ghi cache", async () => {
    fetchMock.mockResolvedValue(new Response("nope", { status: 404 }));
    const event = fetchEvent(new Request("http://bean.test/assets/missing.js"));

    fire("fetch", event);

    await event.calls.respondedWith;
    expect(cachesMock.cachesMap.get("bean-ui-v1")?.size ?? 0).toBe(0);
  });

  it("fetch: /api không bao giờ bị chạm — trình duyệt tự xử", () => {
    for (const path of ["/api/status", "/api/ws", "/api"]) {
      const event = fetchEvent(new Request(`http://bean.test${path}`));

      fire("fetch", event);

      expect(event.calls.respondedWith, path).toBeUndefined();
    }
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("fetch: POST và request ngoài nguồn gốc bị bỏ qua", () => {
    const post = fetchEvent(new Request("http://bean.test/anything", { method: "POST" }));
    fire("fetch", post);
    const crossOrigin = fetchEvent(new Request("https://cdn.example/x.js"));
    fire("fetch", crossOrigin);

    expect(post.calls.respondedWith).toBeUndefined();
    expect(crossOrigin.calls.respondedWith).toBeUndefined();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("fetch: mạng hỏng thì trả bản đã cache", async () => {
    fetchMock.mockRejectedValue(new TypeError("network down"));
    cachesMock.cachesMap.set(
      "bean-ui-v1",
      new Map([["http://bean.test/assets/index.js", new Response("cached")]]),
    );
    const event = fetchEvent(new Request("http://bean.test/assets/index.js"));

    fire("fetch", event);

    const served = await event.calls.respondedWith;
    expect(await served?.text()).toBe("cached");
  });

  // Offline không có mạng: navigation phải nhận được shell đã precache thay vì
  // trang lỗi của trình duyệt — shell báo lỗi API rõ ràng ngay trong app.
  it("fetch: navigate offline thì trả shell index.html", async () => {
    fetchMock.mockRejectedValue(new TypeError("network down"));
    cachesMock.cachesMap.set("bean-ui-v1", new Map([["/index.html", new Response("shell")]]));
    const event = fetchEvent(navigateRequest("http://bean.test/"));

    fire("fetch", event);

    const served = await event.calls.respondedWith;
    expect(await served?.text()).toBe("shell");
  });

  it("fetch: offline mà không có gì trong cache thì để lỗi nổ ra", async () => {
    fetchMock.mockRejectedValue(new TypeError("network down"));
    const event = fetchEvent(new Request("http://bean.test/assets/index.js"));

    fire("fetch", event);

    await expect(event.calls.respondedWith).rejects.toThrow("network down");
  });

  // --- Web Push -------------------------------------------------------------

  it("push: JSON hợp lệ thì hiện notification đúng title/body và giữ url để nhấp", async () => {
    const { event, done } = pushEvent(
      JSON.stringify({ title: "Bean đã trả lời xong", body: "Có câu trả lời mới.", url: "/" }),
    );

    fire("push", event);
    await done();

    expect(fakeSelf.registration.showNotification).toHaveBeenCalledOnce();
    const [title, options] = fakeSelf.registration.showNotification.mock.calls[0];
    expect(title).toBe("Bean đã trả lời xong");
    expect(options).toMatchObject({ body: "Có câu trả lời mới.", data: { url: "/" } });
  });

  it("push: JSON hỏng vẫn hiện notification với tiêu đề mặc định — không chết im", async () => {
    const { event, done } = pushEvent("không phải json");

    fire("push", event);
    await done();

    expect(fakeSelf.registration.showNotification).toHaveBeenCalledOnce();
    const [title, options] = fakeSelf.registration.showNotification.mock.calls[0];
    expect(title).toBe("Bean");
    expect(options?.data).toEqual({ url: "/" });
  });

  it("push: không có dữ liệu (event.data null) vẫn hiện notification", async () => {
    const { event, done } = pushEvent(null);

    fire("push", event);
    await done();

    expect(fakeSelf.registration.showNotification).toHaveBeenCalledOnce();
    expect(fakeSelf.registration.showNotification.mock.calls[0][0]).toBe("Bean");
  });

  it("notificationclick: có cửa sổ mở thì focus, không mở tab mới", async () => {
    const win = { focus: vi.fn() };
    fakeSelf.clients.matchAll.mockResolvedValueOnce([win]);
    const { event, done } = notificationEvent({ url: "/" });

    fire("notificationclick", event);
    await done();

    expect(event.notification.close).toHaveBeenCalledOnce();
    expect(win.focus).toHaveBeenCalledOnce();
    expect(fakeSelf.clients.openWindow).not.toHaveBeenCalled();
  });

  it("notificationclick: không có cửa sổ thì mở url đính kèm trong thông báo", async () => {
    fakeSelf.clients.matchAll.mockResolvedValueOnce([]);
    const { event, done } = notificationEvent({ url: "/?session=1" });

    fire("notificationclick", event);
    await done();

    expect(fakeSelf.clients.openWindow).toHaveBeenCalledWith("/?session=1");
  });

  it("notificationclick: thiếu data thì mở trang gốc", async () => {
    fakeSelf.clients.matchAll.mockResolvedValueOnce([]);
    const { event, done } = notificationEvent(null);

    fire("notificationclick", event);
    await done();

    expect(fakeSelf.clients.openWindow).toHaveBeenCalledWith("/");
  });
});
