/**
 * Web Push (VAPID) phía client: đăng ký subscription Push API của trình duyệt
 * rồi gửi lên server để dispatcher đẩy thông báo khi tab đã đóng (Android/PWA).
 *
 * # Trình tự bật (tất cả trong ngữ cảnh người dùng bấm nút)
 *
 * 1. Xin quyền `Notification` — như thông báo trong tab (`lib/notify.ts`), tự
 *    xin lúc tải trang sẽ bị trình duyệt chặn vĩnh viễn.
 * 2. Lấy public key VAPID từ server (`GET /api/push/key`) — giá trị công khai;
 *    khóa ký JWT (ES256) stays phía server, không bao giờ xuống client.
 * 3. `pushManager.subscribe({ userVisibleOnly: true, applicationServerKey })` —
 *    trình duyệt sinh cặp khóa ECDH và đăng ký với push service của nó (FCM/
 *    Mozilla autopush), trả `endpoint` https.
 * 4. Gửi `{endpoint, keys}` lên server (`POST /api/push/subscribe`) để dispatcher
 *    mã hoá payload theo RFC 8291 và đẩy qua push service.
 *
 * # Giới hạn môi trường (hiển thị ngay trong panel, không để người dùng đoán)
 *
 * Push API chỉ chạy trong context secure (https hoặc localhost). Truy cập qua
 * LAN IP bằng http (D21.x) sẽ không đăng ký được — giới hạn của trình duyệt,
 * không phải lỗi Bean; `pushSupported()` vẫn true nhưng `subscribe()` ném lỗi.
 */

import { ApiRequestError, api } from "@/api/client";

/** Trình duyệt hiện tại có đủ cả ba API để push hoạt động không. */
export function pushSupported(): boolean {
  return (
    typeof window !== "undefined" &&
    "serviceWorker" in navigator &&
    "PushManager" in window &&
    "Notification" in window
  );
}

/**
 * base64url no-pad → bytes — đúng định dạng public key VAPID mà server trả.
 * `atob` nhận cả chuỗi không đủ `=` (bỏ padding là hợp lệ theo RFC 4648 §5),
 * nên không cần tự bù trước khi decode.
 */
export function base64urlToUint8Array(value: string): Uint8Array<ArrayBuffer> {
  const binary = atob(value.replace(/-/g, "+").replace(/_/g, "/"));
  // Cấp phát ArrayBuffer thường (không SharedArrayBuffer) — `applicationServerKey`
  // của Push API đòi `BufferSource` nên kiểu phần tử phải gắn với ArrayBuffer.
  const bytes = new Uint8Array(new ArrayBuffer(binary.length));
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Subscription hiện tại của trình duyệt này — null khi chưa đăng ký hoặc đã gỡ. */
async function currentPushSubscription(): Promise<PushSubscription | null> {
  if (!pushSupported()) return null;
  try {
    const registration = await navigator.serviceWorker.ready;
    return await registration.pushManager.getSubscription();
  } catch {
    // SW chưa sẵn sàng/context không secure: coi như chưa đăng ký, panel vẫn dùng được.
    return null;
  }
}

/** Thiết bị này có đang nhận push không: có subscription + quyền đã cấp. */
export async function isPushEnabled(): Promise<boolean> {
  const subscription = await currentPushSubscription();
  return subscription !== null && Notification.permission === "granted";
}

/**
 * Bật push cho thiết bị này. Trả `"permission_denied"` khi người dùng từ chối
 * quyền (trạng thái cuối, không retry được); lỗi mạng/server ném ra ngoài để
 * panel hiện thông báo. Trả `"enabled"` chỉ khi server đã lưu subscription.
 */
export async function enablePush(): Promise<"enabled" | "permission_denied"> {
  const permission =
    Notification.permission === "granted" ? "granted" : await Notification.requestPermission();
  if (permission !== "granted") return "permission_denied";

  const { public_key } = await api.pushKey();
  const registration = await navigator.serviceWorker.ready;
  const subscription = await registration.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: base64urlToUint8Array(public_key),
  });

  // JSON của subscription luôn có endpoint + keys theo chuẩn Push API; thiếu là
  // trình duyệt sai chuẩn — ném lỗi thay vì lưu dữ liệu rác lên server.
  const json = subscription.toJSON();
  const endpoint = typeof json.endpoint === "string" ? json.endpoint : null;
  const p256dh = json.keys?.p256dh;
  const auth = json.keys?.auth;
  if (endpoint === null || typeof p256dh !== "string" || typeof auth !== "string") {
    throw new Error("push_subscription_invalid");
  }
  await api.pushSubscribe({ endpoint, keys: { p256dh, auth } });
  return "enabled";
}

/**
 * Tắt push: gỡ subscription phía trình duyệt rồi báo server. Server trả 404 khi
 * chưa có bản ghi (đã gỡ từ thiết bị khác) — không coi là lỗi; các lỗi khác ném
 * ra ngoài, còn subscription chết lọt lại server cũng tự bị xoá ở lần gửi đầu
 * (404/410 → delete trong dispatcher).
 */
export async function disablePush(): Promise<void> {
  const subscription = await currentPushSubscription();
  if (subscription) await subscription.unsubscribe();
  const endpoint = subscription?.toJSON().endpoint;
  if (endpoint) {
    try {
      await api.pushUnsubscribe({ endpoint });
    } catch (error) {
      if (!(error instanceof ApiRequestError && error.status === 404)) throw error;
    }
  }
}
