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
        target: "http://127.0.0.1:7878",
        changeOrigin: false,
        ws: true,
      },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: false,
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
