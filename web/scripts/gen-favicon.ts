/**
 * Sinh `public/favicon.svg` từ **đúng** hằng số hình mascot trong
 * `src/components/brand/markPaths.ts`.
 *
 * # Vì sao phải sinh, không vẽ tay
 *
 * Favicon và logo HUB phải là **cùng một hình**. Nếu ai đó sửa mascot ở component
 * mà quên favicon, tab trình duyệt sẽ mang một hình khác — và không ai nhận ra
 * cho tới khi nhìn thấy nó. Sinh từ một nguồn loại bỏ hẳn khả năng lệch.
 *
 * Chạy: `pnpm gen:favicon`. Có test trong `src/features/hub/HubPage.test.tsx`
 * khẳng định file sinh ra khớp hẳng số với component, nên quên chạy lại sẽ
 * **fail test** chứ không im lặng lệch.
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { MARK_STROKE, MARK_VIEWBOX, markPaths } from "../src/components/brand/markPaths.ts";

const here = dirname(fileURLToPath(import.meta.url));
const target = resolve(here, "../public/favicon.svg");

// Nền phải khớp `--surface` (token HUB) để favicon không thành nền đen trên thanh
// công cụ sáng; nét dùng `--ink`. Ở 16–32px chỉ giữ `silhouette` — đúng như
// đặc tả: tai cụp + mũi là ba thứ duy nhất đọc được ở kích thước đó.
const paths = markPaths("silhouette")
  .map((d) => `    <path d="${d}"/>`)
  .join("\n");

const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${MARK_VIEWBOX}" width="32" height="32">
  <rect width="32" height="32" rx="6" fill="#edf1f3"/>
  <g transform="translate(4 4)" fill="none" stroke="#111a1f" stroke-width="${MARK_STROKE}" stroke-linecap="round" stroke-linejoin="round">
${paths}
  </g>
</svg>
`;

mkdirSync(dirname(target), { recursive: true });
writeFileSync(target, svg, "utf8");
console.log(`đã sinh ${target}`);
