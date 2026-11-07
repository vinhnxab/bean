/**
 * Đăng ký service worker cho PWA (docs/decisions.md — "PWA").
 *
 * # Vì sao tách thành module riêng
 *
 * `main.tsx` không được biết tới PWA ngoài một lời gọi: đăng ký là việc phụ
 * (nâng cấp tuỳ chọn), lỗi của nó **không được** làm sập ứng dụng — mọi nhánh
 * thục đều kết thúc im lặng hoặc `console.warn`.
 *
 * # Vì sao chỉ đăng ký ở bản production
 *
 * Dev server của Vite (cổng 5173) không phục vụ `dist/sw.js`, và SW caches ở
 * dev sẽ giữ kết quả cũ của HMR — đúng lớp lỗi "bản cũ mãi" mà mục 22.27 vừa
 * xử lý ở server. Cả hai chế độ phục vụ của Bean (`ui_dir` và bản nhúng) đều
 * trả `/sw.js` với `no-cache`, nên bản build mới luôn được nhận lại.
 */

/** Tùy chọn bơm cho test — mặc định đọc từ `import.meta.env`. */
export interface PwaOptions {
  /** Ghi đè `import.meta.env.PROD`. */
  prod?: boolean;
}

export function registerServiceWorker(options?: PwaOptions): void {
  const prod = options?.prod ?? import.meta.env.PROD;
  if (!prod) return;
  if (typeof window === "undefined") return;
  if (!("serviceWorker" in navigator)) return;

  const register = (): void => {
    navigator.serviceWorker.register("/sw.js").catch((error: unknown) => {
      // Context không secure (truy cập qua LAN IP bằng http) hoặc trình duyệt
      // chặn SW: PWA tắt cho phiên này, chat vẫn chạy bình thường.
      console.warn("Bean: không đăng ký được service worker — PWA tắt cho phiên này.", error);
    });
  };

  // Đợi cửa sổ tải xong: install SW không tranh băng thông với bundle chính.
  if (document.readyState === "complete") {
    register();
  } else {
    window.addEventListener("load", register, { once: true });
  }
}
