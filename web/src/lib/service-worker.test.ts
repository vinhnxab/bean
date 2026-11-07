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
  clients: { claim: vi.fn(async () => []) },
};

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
});
