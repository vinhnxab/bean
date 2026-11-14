import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  canNotifyNow,
  disableNotifications,
  enableNotifications,
  isNotifyEnabled,
  NOTIFY_STORAGE_KEY,
  notificationPermission,
  notificationsSupported,
  OPEN_SESSION_EVENT,
  showDesktopNotification,
} from "@/lib/notify";
import { installFakeNotifications, stubPageContext } from "@/test/notify";

describe("lib/notify — phát hiện API", () => {
  // File này KHÔNG cài fake: đúng trạng thái jsdom thật (không có Notification).
  afterEach(() => {
    window.localStorage.clear();
  });

  it("jsdom không có Notification thì mọi đường đều an toàn (không bắn, không lỗi)", () => {
    expect(notificationsSupported()).toBe(false);
    expect(notificationPermission()).toBe("unsupported");
    expect(canNotifyNow()).toBe(false);
    expect(showDesktopNotification({ title: "x", body: "y", tag: "t", sessionId: 1 })).toBe(false);
  });
});

describe("lib/notify — bật/tắt và xin quyền", () => {
  let fake: ReturnType<typeof installFakeNotifications>;

  beforeEach(() => {
    fake = installFakeNotifications();
    fake.setPermission("default");
    window.localStorage.clear();
  });
  afterEach(() => {
    fake.restore();
    window.localStorage.clear();
  });

  it("bật lần đầu: xin quyền, được cấp thì ghi lựa chọn", async () => {
    fake.setNextRequestResult("granted");

    const permission = await enableNotifications();

    expect(permission).toBe("granted");
    expect(fake.requestPermission).toHaveBeenCalledTimes(1);
    expect(window.localStorage.getItem(NOTIFY_STORAGE_KEY)).toBe("1");
    expect(isNotifyEnabled()).toBe(true);
  });

  it("bị từ chối quyền: KHÔNG ghi lựa chọn, nút sẽ quay về tắt thay vì bật ảo", async () => {
    fake.setNextRequestResult("denied");

    const permission = await enableNotifications();

    expect(permission).toBe("denied");
    expect(window.localStorage.getItem(NOTIFY_STORAGE_KEY)).toBeNull();
    expect(isNotifyEnabled()).toBe(false);
  });

  it("quyền đã cấp sẵn (lần bật sau): không xin lại, ghi lựa chọn ngay", async () => {
    fake.setPermission("granted");

    const permission = await enableNotifications();

    expect(permission).toBe("granted");
    expect(fake.requestPermission).not.toHaveBeenCalled();
    expect(window.localStorage.getItem(NOTIFY_STORAGE_KEY)).toBe("1");
  });

  it("tắt: xoá lựa chọn, không đụng quyền (trình duyệt không cho thu hồi)", async () => {
    fake.setPermission("granted");
    await enableNotifications();
    expect(isNotifyEnabled()).toBe(true);

    disableNotifications();

    expect(isNotifyEnabled()).toBe(false);
    expect(window.localStorage.getItem(NOTIFY_STORAGE_KEY)).toBeNull();
  });
});

describe("lib/notify — điều kiện bắn", () => {
  let fake: ReturnType<typeof installFakeNotifications>;

  beforeEach(() => {
    fake = installFakeNotifications();
    fake.setPermission("granted");
    window.localStorage.clear();
  });
  afterEach(() => {
    fake.restore();
  });

  it("đủ điều kiện: đã bật + quyền granted + tab ẩn", () => {
    window.localStorage.setItem(NOTIFY_STORAGE_KEY, "1");
    const restoreContext = stubPageContext({ visibility: "hidden", focused: false });

    try {
      expect(canNotifyNow()).toBe(true);
    } finally {
      restoreContext();
    }
  });

  it("tab đang hiển thị và giữ focus thì không bắn — người dùng đang nhìn màn hình", () => {
    window.localStorage.setItem(NOTIFY_STORAGE_KEY, "1");
    const restoreContext = stubPageContext({ visibility: "visible", focused: true });

    try {
      expect(canNotifyNow()).toBe(false);
    } finally {
      restoreContext();
    }
  });

  it("tab hiển thị nhưng mất focus (cửa sổ khác) vẫn bắn", () => {
    window.localStorage.setItem(NOTIFY_STORAGE_KEY, "1");
    const restoreContext = stubPageContext({ visibility: "visible", focused: false });

    try {
      expect(canNotifyNow()).toBe(true);
    } finally {
      restoreContext();
    }
  });

  it("chưa bật lựa chọn thì không bắn dù quyền còn nguyên", () => {
    const restoreContext = stubPageContext({ visibility: "hidden" });

    try {
      expect(canNotifyNow()).toBe(false);
    } finally {
      restoreContext();
    }
  });
});

describe("lib/notify — bắn thông báo", () => {
  let fake: ReturnType<typeof installFakeNotifications>;

  beforeEach(() => {
    fake = installFakeNotifications();
    fake.setPermission("granted");
    window.localStorage.setItem(NOTIFY_STORAGE_KEY, "1");
  });
  afterEach(() => {
    fake.restore();
    window.localStorage.clear();
  });

  function withHiddenTab(run: () => void): void {
    const restoreContext = stubPageContext({ visibility: "hidden", focused: false });
    try {
      run();
    } finally {
      restoreContext();
    }
  }

  it("bắn đúng tiêu đề, body, tag và icon PWA", () => {
    withHiddenTab(() => {
      const fired = showDesktopNotification({
        title: "Tiêu đề",
        body: "Nội dung",
        tag: "bean:test:1",
        sessionId: 7,
      });

      expect(fired).toBe(true);
      expect(fake.instances).toHaveLength(1);
      expect(fake.instances[0].title).toBe("Tiêu đề");
      expect(fake.instances[0].body).toBe("Nội dung");
      expect(fake.instances[0].tag).toBe("bean:test:1");
      expect(fake.instances[0].icon).toBe("/pwa-192.png");
    });
  });

  it("nhấp thông báo: focus cửa sổ, đóng thông báo, phát sự kiện mở đúng phiên", () => {
    withHiddenTab(() => {
      const focus = vi.spyOn(window, "focus").mockReturnValue(undefined);
      const opened: number[] = [];
      const listener = (event: Event) => opened.push((event as CustomEvent<number>).detail);
      window.addEventListener(OPEN_SESSION_EVENT, listener);

      try {
        showDesktopNotification({ title: "T", body: "B", tag: "tag", sessionId: 42 });
        fake.instances[0].onclick?.();

        expect(focus).toHaveBeenCalledTimes(1);
        expect(fake.instances[0].close).toHaveBeenCalledTimes(1);
        expect(opened).toEqual([42]);
      } finally {
        window.removeEventListener(OPEN_SESSION_EVENT, listener);
        focus.mockRestore();
      }
    });
  });

  it("thông báo lỗi toàn cục (không có phiên): nhấp chỉ focus, không phát sự kiện", () => {
    withHiddenTab(() => {
      const listener = vi.fn();
      window.addEventListener(OPEN_SESSION_EVENT, listener);

      try {
        showDesktopNotification({ title: "T", body: "B", tag: "tag", sessionId: null });
        fake.instances[0].onclick?.();

        expect(listener).not.toHaveBeenCalled();
      } finally {
        window.removeEventListener(OPEN_SESSION_EVENT, listener);
      }
    });
  });

  it("điều kiện chưa đủ thì không tạo instance nào", () => {
    // Tab đang hiển thị + giữ focus: `canNotifyNow` sai. (jsdom mặc định trả
    // `hasFocus() = false` — phải stub tường minh mới đúng ngữ cảnh thật.)
    const restoreContext = stubPageContext({ visibility: "visible", focused: true });
    try {
      const fired = showDesktopNotification({ title: "T", body: "B", tag: "tag", sessionId: 1 });
      expect(fired).toBe(false);
      expect(fake.instances).toHaveLength(0);
    } finally {
      restoreContext();
    }
  });
});
