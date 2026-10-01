/**
 * Sinh ảnh mascot từ **nguồn duy nhất** `web/brand/bean.png`.
 *
 * # Vì sao sinh, không copy thẳng
 *
 * `bean.png` gốc là ảnh vuông 500×500, nặng ~264 KB — quá nặng để nhúng vào mọi
 * avatar trong HTML, và nhúng thẳng sẽ khiến UI tải 264 KB chỉ để vẽ một chú
 * chó 32px. Nên file này **sinh ra** đúng những kích thước UI thật sự dùng.
 *
 * # Vì sao resize bằng Chrome, không thêm thư viện ảnh
 *
 * Dự án cấm thêm dependency không cần thiết (agents.md mục 15.10). `sharp`/`jimp`
 * chỉ phục vụ đúng một việc resize, còn `puppeteer-core` **đã có sẵn** trong
 * devDependencies để chụp ảnh giao diện. Dùng canvas của Chrome: không thêm gói,
 * và thuật toán resize là của chính trình duyệt sẽ hiển thị ảnh.
 *
 * Chạy: `node --experimental-strip-types scripts/gen-brand-assets.ts`
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer-core";

import { BADGE_BG, BG_PROBE_POINTS, BG_TOLERANCE, SOURCE_EDGE } from "../src/components/brand/badge.ts";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE = resolve(ROOT, "brand/bean.png");

/**
 * Kích thước sinh ra — mỗi cái đều ứng với một chỗ dùng thật trong UI.
 *
 * **Không mục nào bị cắt, không mục nào bị bo**: mọi kích thước chỉ **thu nhỏ
 * trọn ảnh gốc** về cạnh tương ứng. Chú chó, bong bóng "?" và vòng viền luôn còn
 * nguyên ở mọi cỡ — avatar 28px trong chat và avatar 56px ở màn đăng nhập là
 * **cùng một hình**, chỉ khác tỉ lệ. Bốn góc ảnh gốc trong suốt (đo được:
 * alpha=0) nên ảnh giữ nguyên hình tròn của huy hiệu mà **không cần mặt nạ**.
 * Trước đây avatar cắt cúp vào mặt; bản này bỏ hẳn bước crop.
 *
 * `fill: true` (chỉ `apple-touch-icon`) lót kín nền be: iOS đổ nền trong suốt
 * thành đen, mà icon màn hình chính cần nền đặc, không thể trong suốt.
 */
const TARGETS = [
  // Avatar: màn đăng nhập (56px), logo HUB (44px), tin nhắn trong chat (28–32px).
  // Bản lớn 128×128 (×2 cho Retina) dùng cho avatar ≥ 44px.
  { file: "bean-avatar.png", size: 128, fill: false },
  // Bản nhỏ 64×64: một phiên chat dài có hàng trăm avatar, và mỗi bản 128×128
  // giải mã thành 64 KB trong RAM — 64×64 giảm 4 lần bộ nhớ, vẫn sắc ở 32px trên
  // màn hình Retina 2x. Chính bản này cũng là **favicon** (xem `index.html`):
  // huy hiệu tròn là hình dạng đúng cho một tab trình duyệt.
  { file: "bean-avatar-sm.png", size: 64, fill: false },
  // iOS **không** dùng SVG cho icon màn hình chính, bỏ qua mọi kích thước khác
  // 180, và **nền trong suốt bị đổ thành đen** — icon sẽ là huy hiệu tròn trên ô
  // vuông đen. Nên phải lót kín bằng màu nền của chính huy hiệu.
  { file: "apple-touch-icon.png", size: 180, fill: true },
] as const;

const CHROME = "/usr/bin/google-chrome";

async function main(): Promise<void> {
  const bytes = await readFile(SOURCE);
  const browser = await puppeteer.launch({
    executablePath: CHROME,
    args: ["--no-sandbox", "--disable-dev-shm-usage"],
  });
  try {
    const page = await browser.newPage();
    for (const target of TARGETS) {
      // Thu nhỏ ảnh gốc rồi vẽ vào canvas, xuất PNG: trình duyệt tự resize, không
      // cần thư viện ảnh. `imageSmoothingQuality = "high"` để lông xoăn không bị
      // vỡ bậc khi thu từ 500px xuống 28px.
      const dataUrl = await page.evaluate(
        async (
          base64: string,
          size: number,
          edge: number,
          fileName: string,
          fill: boolean,
          probePoints: readonly (readonly [number, number])[],
          expectedBg: { r: number; g: number; b: number },
          tolerance: number,
        ) => {
          const image = new Image();
          image.src = `data:image/png;base64,${base64}`;
          await image.decode();
          // Đo ảnh thật, đừng so với hằng số: nếu bạn thay `bean.png` bằng ảnh
          // cỡ khác, mọi kích thước sẽ lệch mà không có gì báo sai. Cảnh báo ngay.
          if (image.naturalWidth !== edge || image.naturalHeight !== edge) {
            throw new Error(
              `Ảnh gốc là ${image.naturalWidth}×${image.naturalHeight}, ` +
                `không phải ${edge}×${edge}. Cập nhật SOURCE_EDGE trong badge.ts.`,
            );
          }
          const canvas = document.createElement("canvas");
          canvas.width = size;
          canvas.height = size;
          const ctx = canvas.getContext("2d");
          if (!ctx) throw new Error("không lấy được canvas 2d");
          ctx.imageSmoothingEnabled = true;
          ctx.imageSmoothingQuality = "high";

          if (fill) {
            // Trung vị **phần sáng** trong nhiều điểm dò. Một điểm đơn lẻ thì
            // dễ rơi vào lông chó (bản đầu lấy `(250,110)` và lót icon bằng nâu
            // sẫm); nhiều điểm + trung vị thì chỉ còn màu nền.
            const rgb = (() => {
              const t = document.createElement("canvas");
              t.width = edge;
              t.height = edge;
              const tc = t.getContext("2d", { willReadFrequently: true });
              if (!tc) throw new Error("không lấy được canvas lấy màu");
              tc.drawImage(image, 0, 0);
              // Lọc theo **độ sáng**, không chỉ theo alpha: cả lông chó lẫn nền
              // đều đục. Nền huy hiệu sáng (lum ~215), lông tối (lum ~25). Bỏ qua
              // bước lọc này thì trung vị rơi vào lông — đã xảy ra thật.
              const bright: number[][] = [];
              for (const [px, py] of probePoints) {
                const d = tc.getImageData(px, py, 1, 1).data;
                const lum = 0.299 * (d[0] ?? 0) + 0.587 * (d[1] ?? 0) + 0.114 * (d[2] ?? 0);
                if ((d[3] ?? 0) > 200 && lum > 140) {
                  bright.push([d[0] ?? 0, d[1] ?? 0, d[2] ?? 0]);
                }
              }
              if (bright.length < 2) {
                throw new Error(
                  `Chỉ dò được ${bright.length} điểm sáng; điểm dò trong badge.ts không còn nằm trên nền huy hiệu.`,
                );
              }
              const median = (i: number) => {
                const v = bright.map((c) => c[i] ?? 0).sort((a, b) => a - b);
                return v[Math.floor(v.length / 2)] ?? 0;
              };
              const rgb = [median(0), median(1), median(2)];
              // So với hằng số đã khai: thay ảnh là lệch, và lệch thì phải báo
              // chứ không lặng lẽ đổi màu nền dưới chân component.
              const drift = Math.max(
                ...rgb.map((v, i) => Math.abs(v - [expectedBg.r, expectedBg.g, expectedBg.b][i] ?? 0)),
              );
              if (drift > tolerance) {
                throw new Error(
                  `Màu nền huy hiệu đo được rgb(${rgb.join(", ")}) lệch ${drift} ` +
                    `> ${tolerance} so với BADGE_BG đã khai. Cần cập nhật badge.ts.`,
                );
              }
              return `rgb(${rgb.join(", ")})`;
            })();
            ctx.fillStyle = rgb;
            ctx.fillRect(0, 0, size, size);
          }

          // Thu nhỏ **trọn** ảnh gốc: không cắt, chỉ scale. Không cần mặt nạ bo
          // tròn — huy hiệu vốn là hình tròn trên nền trong suốt, nên chỉ scale
          // là giữ nguyên hình dạng.
          ctx.drawImage(image, 0, 0, edge, edge, 0, 0, size, size);

          // Tự kiểm tra kết quả: đọc alpha ở 4 góc **từ chính ảnh vừa xuất**.
          // Sai ở đây không hề báo lỗi — generator vẫn chạy, file vẫn sinh ra,
          // không test nào đỏ; chỉ có mắt người thấy. Nên kiểm ở đây, ngay chỗ
          // có đủ ngữ cảnh để biết cái gì là đúng.
          const check = document.createElement("canvas");
          check.width = size;
          check.height = size;
          const cctx = check.getContext("2d", { willReadFrequently: true });
          if (!cctx) throw new Error("không lấy được canvas kiểm tra");
          const url = canvas.toDataURL("image/png");
          const probe = new Image();
          probe.src = url;
          await probe.decode();
          cctx.drawImage(probe, 0, 0);
          const corner = (x: number, y: number) =>
            cctx.getImageData(x, y, 1, 1).data[3] ?? 0;
          const corners = [corner(0, 0), corner(size - 1, 0), corner(0, size - 1), corner(size - 1, size - 1)];
          // Ảnh gốc để góc **trong suốt** (huy hiệu tròn trên nền trong suốt);
          // riêng `apple-touch-icon` có lót nền nên góc phải **đặc**. Lệch một
          // trong hai nghĩa là `bean.png` đã đổi (thêm nền) hoặc lớp lót bị lệch.
          const uniform = fill ? corners.every((a) => a > 247) : corners.every((a) => a < 8);
          if (!uniform) {
            throw new Error(
              `${fileName}: góc không như mong đợi (alpha=${corners.join(",")}, fill=${fill})`,
            );
          }
          return url;
        },
        bytes.toString("base64"),
        target.size,
        SOURCE_EDGE,
        target.file,
        target.fill,
        BG_PROBE_POINTS,
        BADGE_BG,
        BG_TOLERANCE,
      );
      const out = resolve(ROOT, "public", target.file);
      await mkdir(dirname(out), { recursive: true });
      await writeFile(out, Buffer.from(dataUrl.split(",")[1] ?? "", "base64"));
      console.log(`✓ public/${target.file} (${target.size}×${target.size})`);
    }
  } finally {
    await browser.close();
  }
}

await main();
