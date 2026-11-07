import { afterEach, describe, expect, it, vi } from "vitest";

import { registerServiceWorker } from "@/pwa";

/**
 * Đăng ký PWA là việc phụ của app: sai ở đây không được làm sập chat. Các
 * nhánh đều kiểm bằng giả lập `navigator` (jsdom không có serviceWorker).
 */

/** Navigator giả có `serviceWorker` — thứ jsdom không cung cấp. */
function fakeNavigator(register: ReturnType<typeof vi.fn>) {
  return { serviceWorker: { register } };
}

describe("registerServiceWorker", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("không đăng ký khi không phải bản production", () => {
    const register = vi.fn(() => Promise.resolve({}));
    vi.stubGlobal("navigator", fakeNavigator(register));

    registerServiceWorker({ prod: false });

    expect(register).not.toHaveBeenCalled();
  });

  it("không đăng ký khi trình duyệt không hỗ trợ serviceWorker", () => {
    vi.stubGlobal("navigator", {});

    expect(() => registerServiceWorker({ prod: true })).not.toThrow();
  });

  it("tài liệu đã tải xong thì đăng ký /sw.js ngay", () => {
    const register = vi.fn(() => Promise.resolve({}));
    vi.stubGlobal("navigator", fakeNavigator(register));
    vi.spyOn(document, "readyState", "get").mockReturnValue("complete");

    registerServiceWorker({ prod: true });

    expect(register).toHaveBeenCalledExactlyOnceWith("/sw.js");
  });

  it("tài liệu đang tải thì chờ sự kiện load rồi mới đăng ký", () => {
    const register = vi.fn(() => Promise.resolve({}));
    vi.stubGlobal("navigator", fakeNavigator(register));
    vi.spyOn(document, "readyState", "get").mockReturnValue("loading");
    const listeners = new Map<string, () => void>();
    vi.spyOn(window, "addEventListener").mockImplementation(
      (type: string, listener: EventListenerOrEventListenerObject) => {
        listeners.set(type, listener as () => void);
      },
    );

    registerServiceWorker({ prod: true });
    expect(register).not.toHaveBeenCalled();

    listeners.get("load")?.();
    expect(register).toHaveBeenCalledExactlyOnceWith("/sw.js");
  });

  // Truy cập qua LAN IP bằng http (context không secure) là trường hợp thật
  // của Bean: SW bị chặn — chỉ được warn, tuyệt đối không throw ra ngoài.
  it("lỗi đăng ký bị nuốt thành console.warn, không văng ra ngoài", async () => {
    const register = vi.fn(() => Promise.reject(new Error("insecure context")));
    vi.stubGlobal("navigator", fakeNavigator(register));
    vi.spyOn(document, "readyState", "get").mockReturnValue("complete");
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    registerServiceWorker({ prod: true });
    await Promise.resolve();

    expect(warn).toHaveBeenCalledOnce();
  });
});
