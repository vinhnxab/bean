/**
 * Bean PWA service worker (docs/decisions.md — "PWA").
 *
 * # Chiến lược: network-first, cache chỉ là phương án offline
 *
 * Tên asset **không hash** (web/vite.config.ts) và server trả `no-cache` + ETag
 * cho mọi file UI (mục 11.3, lỗi thật 22.27). Vì vậy service worker CẤM
 * cache-first/stale-while-revalidate: cache sẽ giữ bản UI cũ mãi ngay khi tên
 * file không đổi — tái diễn đúng lỗi đó, nhưng lần này nằm ở client. Với
 * network-first, trình duyệt vẫn revalidate qua ETag như thường (304 rẻ), chỉ
 * khi MẠNG HỎNG mới dùng bản cache.
 *
 * `/api` (REST + WebSocket) không bao giờ bị chạm vào: chat phải đi mạng trực
 * tiếp, và tin nhắn không có ý nghĩa gì khi offline.
 */
const CACHE = "bean-ui-v1";
const OFFLINE_URL = "/index.html";

self.addEventListener("install", (event) => {
  event.waitUntil(
    (async () => {
      // Chỉ precache shell — đủ để offline hiện giao diện và báo lỗi mạng rõ
      // ràng, không ôm cả bundle/font vào cache khi đa số phiên là online.
      const cache = await caches.open(CACHE);
      await cache.add(OFFLINE_URL);
      await self.skipWaiting();
    })(),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      for (const key of await caches.keys()) {
        if (key !== CACHE) await caches.delete(key);
      }
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;
  if (url.pathname === "/api" || url.pathname.startsWith("/api/")) return;
  // Range request (media): trả từ cache sẽ sai nội dung — bỏ qua hoàn toàn.
  if (request.headers.has("range")) return;

  event.respondWith(
    (async () => {
      try {
        const response = await fetch(request);
        if (response.ok) {
          // Không await: không delay response cho người dùng; lỗi ghi cache
          // (hiếm) chỉ là mất phương án offline, không đáng chặn phản hồi.
          const cache = await caches.open(CACHE);
          cache.put(request, response.clone()).catch(() => {});
        }
        return response;
      } catch (error) {
        const cached = await caches.match(request);
        if (cached) return cached;
        if (request.mode === "navigate") {
          const offline = await caches.match(OFFLINE_URL);
          if (offline) return offline;
        }
        throw error;
      }
    })(),
  );
});

// --- Web Push (VAPID) -------------------------------------------------------
// Server mã hoá payload theo RFC 8291 và push service (FCM/Mozilla autopush)
// giải hộ; tới đây trình duyệt nhận JSON {title, body, url}. JSON hỏng hoặc
// rỗng vẫn phải hiện thông báo chung: Chrome CHẾT subscription nếu một push
// không hiện gì (userVisibleOnly), nên tuyệt đối không bỏ qua sự kiện này.

self.addEventListener("push", (event) => {
  let title = "Bean";
  let body = "";
  let url = "/";
  try {
    const payload = JSON.parse(event.data ? event.data.text() : "{}");
    if (typeof payload.title === "string" && payload.title) title = payload.title.slice(0, 100);
    if (typeof payload.body === "string") body = payload.body.slice(0, 300);
    if (typeof payload.url === "string" && payload.url) url = payload.url;
  } catch {
    // Payload hỏng: giữ tiêu đề mặc định — thông báo vẫn phải hiện.
  }
  event.waitUntil(
    self.registration.showNotification(title, {
      body,
      icon: "/pwa-192.png",
      badge: "/pwa-192.png",
      tag: "bean",
      data: { url },
    }),
  );
});

// Nhấp thông báo: tập trung cửa sổ Bean đang mở, không có thì mở theo url đính
// kèm. Không navigate: chạy vào SPA đủ tốt vì url luôn là đường dẫn trong app.

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = event.notification.data && event.notification.data.url ? event.notification.data.url : "/";
  event.waitUntil(
    (async () => {
      const clientList = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
      for (const client of clientList) {
        await client.focus();
        return;
      }
      if (self.clients.openWindow) await self.clients.openWindow(url);
    })(),
  );
});
