/**
 * Thông báo hệ thống (Web Notifications API) khi Bean có việc cần người dùng
 * nhìn: trả lời xong, cần xác nhận hành động, tin chủ động (scheduler/learning),
 * run lỗi.
 *
 * # Vì sao chỉ bắn khi tab đang ẩn
 *
 * Người dùng đang nhìn màn hình chat thì thông báo hệ thống chỉ là nhiễu — UI
 * đã có pill "Đang suy nghĩ", thẻ xác nhận ngay trong dòng thời gian. Thông báo
 * chỉ có giá trị khi họ đang ở tab/cửa sổ khác, nên `canNotifyNow()` chặn mọi
 * trường hợp còn lại. Nhờ vậy việc bật thông báo không cần cơ chế dồn/khoá:
 * mỗi sự kiện đáng thông báo sẽ bắn đúng một lần duy nhất.
 *
 * # Vì sao bật qua nút chuông (user gesture) chứ không tự xin quyền
 *
 * Trình duyệt chỉ cho phép `requestPermission()` trong ngữ cảnh người dùng chủ
 * động; tự xin lúc mở trang là cách nhanh nhất bị chặn vĩnh viễn. Nút chuông ở
 * TopNav là ngữ cảnh đó — bấm mới xin; bị chặn thì lựa chọn không được ghi và
 * nút giữ trạng thái tắt.
 *
 * # Vì sao lưu lựa chọn ở `localStorage` thay vì server
 *
 * Đây là tuỳ chọn của **thiết bị này** (máy tính bật, điện thoại có thể tắt),
 * không phải tuỳ chọn tài khoản — cùng logic với chủ đề (`bean.theme`) và
 * thu gọn sidebar.
 */

/** Khoá lưu lựa chọn bật/tắt thông báo trên thiết bị hiện tại. */
export const NOTIFY_STORAGE_KEY = "bean.notify";

/** Khoá 1 nếu đã bật; xoá hẳn khi tắt — đọc được "chưa từng bật" và "đã tắt" là một. */

export function notificationsSupported(): boolean {
  return typeof window !== "undefined" && "Notification" in window;
}

export type NotifyPermission = "default" | "granted" | "denied" | "unsupported";

export function notificationPermission(): NotifyPermission {
  if (!notificationsSupported()) return "unsupported";
  return Notification.permission;
}

/** Lựa chọn chỉ có nghĩa khi quyền thật sự được cấp — đọc trực tiếp, không cache. */
export function isNotifyEnabled(): boolean {
  try {
    return window.localStorage.getItem(NOTIFY_STORAGE_KEY) === "1";
  } catch {
    // Chế độ riêng tư/storage bị chặn: coi như chưa bật, không báo lỗi ra UI.
    return false;
  }
}

function setStored(enabled: boolean): void {
  try {
    if (enabled) window.localStorage.setItem(NOTIFY_STORAGE_KEY, "1");
    else window.localStorage.removeItem(NOTIFY_STORAGE_KEY);
  } catch {
    // Không lưu được thì lựa chọn chỉ sống trong phiên hiện tại — chấp nhận được.
  }
}

/**
 * Bật thông báo: xin quyền (phải nằm trong handler của thao tác người dùng) và
 * ghi lựa chọn chỉ khi quyền là `granted`. Trả quyền cuối để nút chuông cập
 * trạng thái theo kết quả thật thay vì kết quả mong đợi.
 */
export async function enableNotifications(): Promise<NotifyPermission> {
  if (!notificationsSupported()) return "unsupported";
  const permission: NotifyPermission =
    Notification.permission === "granted" ? "granted" : await Notification.requestPermission();
  setStored(permission === "granted");
  return permission;
}

/** Tắt thông báo: chỉ gỡ lựa chọn, không đụng quyền (trình duyệt không cho thu hồi). */
export function disableNotifications(): void {
  setStored(false);
}

/**
 * Có được phép bắn ngay bây giờ không: đã bật + quyền `granted` + tab **không**
 * đang hiển thị trước mắt người dùng (ẩn tab, cửa sổ khác chiếm focus).
 */
export function canNotifyNow(): boolean {
  return (
    notificationsSupported() &&
    isNotifyEnabled() &&
    Notification.permission === "granted" &&
    (document.visibilityState !== "visible" || !document.hasFocus())
  );
}

export type DesktopNotification = {
  title: string;
  body: string;
  /** Gom nhóm: thông báo cùng `tag` thay thế nhau thay vì chất đống. */
  tag: string;
  /** Phiên để mở khi nhấp; `null` với lỗi toàn cục — nhấp chỉ focus cửa sổ. */
  sessionId: number | null;
};

/** Sự kiện mở đúng phiên khi người dùng nhấp vào thông báo (App.tsx lắng nghe). */
export const OPEN_SESSION_EVENT = "bean:open-session";

/** Bắn thông báo; trả `true` nếu đã bắn thật. Nhấp vào thông báo: focus cửa sổ + mở phiên. */
export function showDesktopNotification(content: DesktopNotification): boolean {
  if (!canNotifyNow()) return false;
  const notification = new Notification(content.title, {
    body: content.body,
    tag: content.tag,
    icon: "/pwa-192.png",
  });
  notification.onclick = () => {
    notification.close();
    window.focus();
    if (content.sessionId !== null) {
      window.dispatchEvent(new CustomEvent<number>(OPEN_SESSION_EVENT, { detail: content.sessionId }));
    }
  };
  return true;
}
