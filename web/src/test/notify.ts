import { vi } from "vitest";

/**
 * Fake `Notification` cho jsdom (jsdom KHÔNG có Web Notifications API).
 *
 * Dùng chung cho test của `lib/notify` và `features/chat/notifyEvents`: thay
 * `window.Notification` bằng một lớp giả giữ đúng hình dáng API thật (quyền là
 * thuộc tính tĩnh, `requestPermission` trả Promise, instance có `close`/`onclick`),
 * để code sản phẩm chạy thật trên đó thay vì bị mock lọt qua everywhere.
 *
 * # Quy ước khôi phục
 *
 * Mỗi file test gọi `installFakeNotifications()` trong `beforeEach` và
 * `restore()` trong `afterEach` — không dùng `vi.stubGlobal` vì đó là mock toàn
 * cục cần `unstubAllGlobals`, còn descriptor gốc của jsdom thì giữ được chính xác.
 */
export type FakePermission = "default" | "granted" | "denied";

export type FakeNotificationInstance = {
  title: string;
  body: string;
  tag: string | undefined;
  icon: string | undefined;
  close: ReturnType<typeof vi.fn>;
  onclick: (() => void) | null;
};

export function installFakeNotifications() {
  const permission: { current: FakePermission } = { current: "default" };
  const requestResult: { current: FakePermission } = { current: "granted" };
  const instances: FakeNotificationInstance[] = [];
  const requestPermission = vi.fn(async (): Promise<FakePermission> => requestResult.current);

  class FakeNotification {
    static get permission(): FakePermission {
      return permission.current;
    }
    static requestPermission = requestPermission;

    title: string;
    body: string;
    tag: string | undefined;
    icon: string | undefined;
    close = vi.fn();
    onclick: (() => void) | null = null;

    constructor(title: string, options?: { body?: string; tag?: string; icon?: string }) {
      this.title = title;
      this.body = options?.body ?? "";
      this.tag = options?.tag;
      this.icon = options?.icon;
      instances.push(this);
    }
  }

  const original = Object.getOwnPropertyDescriptor(window, "Notification");
  Object.defineProperty(window, "Notification", {
    configurable: true,
    writable: true,
    value: FakeNotification,
  });

  return {
    instances,
    requestPermission,
    setPermission: (value: FakePermission) => {
      permission.current = value;
    },
    setNextRequestResult: (value: FakePermission) => {
      requestResult.current = value;
    },
    restore: () => {
      if (original) Object.defineProperty(window, "Notification", original);
      else delete (window as { Notification?: unknown }).Notification;
    },
  };
}

/**
 * Chố trạng thái trang cho `canNotifyNow()`: jsdom mặc định `visibilityState`
 * là "visible" và `hasFocus()` trả `true` — tức tab đang nhìn, không bắn được.
 * Override bằng spy để test cả nhánh "tab ẩn".
 */
export function stubPageContext({
  visibility = "visible",
  focused = true,
}: {
  visibility?: DocumentVisibilityState;
  focused?: boolean;
} = {}) {
  const visibilitySpy = vi.spyOn(document, "visibilityState", "get").mockReturnValue(visibility);
  const focusSpy = vi.spyOn(document, "hasFocus").mockReturnValue(focused);
  return () => {
    visibilitySpy.mockRestore();
    focusSpy.mockRestore();
  };
}
