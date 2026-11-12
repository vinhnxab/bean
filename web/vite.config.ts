/// <reference types="vitest/config" />
import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Bean web (agents.md mục 3.2, 12.2).
// - Dev: proxy `/api` (bao gồm WebSocket `/api/ws`) sang `bean serve` ở 127.0.0.1:7878.
// - Không tải bất kỳ tài nguyên nào từ CDN/domain ngoài (mục 0.9).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },
  server: {
    port: 5173,
    proxy: {
      "/api": {
        // Backend dev chạy bằng cấu hình riêng (`bean.dev.toml`) ở cổng **7879**,
        // vì Bean kiểm tra `Origin` phải khớp `web.public_origin`: muốn xem giao
        // diện ở `http://localhost:5173` thì `public_origin` của instance dev
        // phải là chính `http://localhost:5173`. Instance thật (cổng 7878,
        // `public_origin` 127.0.0.1:7878) không bị ảnh hưởng và không đụng
        // `data.dir` của nó.
        target: "http://127.0.0.1:7879",
        // GIỮ `false`: `Origin` của trình duyệt (localhost:5173) phải đi nguyên
        // tới backend để nó tự so khớp với `public_origin`. Đổi thành `true` sẽ
        // ép `Origin` về giá trị của `target` — che đúng cái rào CSRF mà
        // `public_origin` dựng ra.
        changeOrigin: false,
        ws: true,
      },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: false,
    // Font (woff2/ttf) KHÔNG bao giờ inline thành `data:` URI (K31): CSP tại
    // `bean-web` (security_headers) không khai `font-src data:` — chủ ý giữ chặt
    // theo mục 15.7 — nên font inline của chính Bean bị chặn trong console. Mặc
    // định Vite inline mọi asset < 4096 byte, và các subset font nhỏ (cyrillic,
    // greek-ext…) rơi vào ngưỡng đó. Hàm này giữ nguyên hành vi inline cho asset
    // khác (SVG nhỏ…), chỉ đẩy font ra file riêng — cùng origin nên vẫn nằm
    // trong `default-src 'self'`, không phải nới CSP.
    assetsInlineLimit(filePath, content) {
      if (/\.(woff2?|ttf|otf|eot)$/i.test(filePath)) return false;
      return content.length < 4096;
    },
    // Không hash tên file đầu ra: index.html luôn trỏ tới /assets/index.js +
    // /assets/index.css, nên việc chép web/dist mới vào [web].ui_dir (hoặc rebuild
    // Rust nhúng) không phải đổi tên asset trong backend. Trade-off: các asset
    // không còn hash nên browser có thể cache phiên bản cũ — khi cập nhật UI hãy
    // xóa web/dist trước khi build, hoặc phục vụ UI từ đĩa và restart Bean.
    rollupOptions: {
      output: {
        entryFileNames: "assets/index.js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: "assets/[name][extname]",
      },
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    css: false,
    // jsdom được dựng lại cho từng test file (~10s với 12 file), nên mặc định
    // `testTimeout` 5s của Vitest làm các test nặng chập chờn fail giả ("Test timed out
    // in 5000ms") tuỳ theo tải máy. Nới timeout và giới hạn worker để `make check`
    // ổn định. `maxWorkers` là option top-level (Vitest 4+ đã bỏ `poolOptions`).
    testTimeout: 30_000,
    maxWorkers: 4,
  },
});
