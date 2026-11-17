import { expect, type Page, test } from "@playwright/test";

/**
 * E2E UI bằng browser thật (mục 3, M16) — đúng luồng M10: đăng nhập → gửi tin →
 * thẻ xác nhận → duyệt → câu trả lời cuối, qua WebSocket thật của Chrome.
 *
 * # Bất biến về kịch bản
 *
 * FakeProvider tiêu thụ `fake-script.json` tuần tự toàn cục, nên file này chạy
 * tuần tự (`workers: 1` trong playwright.config) và mỗi test dùng đúng phần
 * kịch bản của mình:
 *   1. journey desktop     → response 0 (text) + 1,2 (write_file + final)
 *   2. viewport mobile A3  → response 3 (text)
 *
 * # Selector
 *
 * Bám vào nhãn i18n `vi` (aria-label của composer, "Đăng nhập", "Cho phép") và
 * `role="alertdialog"` của ConfirmCard — đó là những gì người dùng thật chạm,
 * không bám class Tailwind dễ đổi.
 */

const PASSWORD = "e2e-password-123456";
// i18n hiện dùng `DEFAULT_LANG = "en"` (web/src/i18n/index.tsx) — selector bám
// nhãn **thật** mà người dùng thấy, không bám class Tailwind dễ đổi.
const COMPOSER = "Type your message…";

/** Đăng nhập qua form thật (cookie thật), tạo session bằng REST rồi mở trang chat. */
async function loginAndOpenSession(page: Page): Promise<number> {
  await page.goto("/");
  await expect(page).toHaveURL(/\/login/);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.waitForURL(/\/$/);

  // Tạo session qua `request` của context (chia sẻ cookie với page). Server đòi
  // Origin khớp `public_origin` cho mọi request thay đổi dữ liệu.
  const response = await page.request.post("/api/sessions", {
    data: { title: null },
    headers: { Origin: new URL(page.url()).origin },
  });
  expect(response.ok()).toBeTruthy();
  const session = (await response.json()) as { id: number };

  await page.goto(`/sessions/${session.id}`);
  await page.waitForURL(new RegExp(`/sessions/${session.id}`));
  return session.id;
}

test.describe
  .serial("Bean UI — browser thật, fake-llm", () => {
    test("đăng nhập, chat và duyệt hành động write_file (M10)", async ({ page }) => {
      const sessionId = await loginAndOpenSession(page);
      expect(sessionId).toBeGreaterThan(0);

      // Composer chỉ bật khi WebSocket đã "connected" (ChatPage disable khi chưa).
      const composer = page.getByLabel(COMPOSER);
      await expect(composer).toBeEnabled();
      const send = page.getByRole("button", { name: "Send" });

      // 1) Lượt chat thuần — response 0.
      await composer.fill("xin chào");
      await send.click();
      await expect(page.getByText("Đây là câu trả lời E2E cho tin nhắn đầu tiên")).toBeVisible({
        timeout: 30_000,
      });

      // 2) Lượt cần duyệt — response 1 (write_file) rồi 2 (final). ConfirmCard dùng
      //    role="alertdialog"; nút "Cho phép" bám đúng nhãn i18n, exact để không
      //    trùng "Cho phép trong phiên".
      await composer.fill("ghi file giúp tôi");
      await send.click();
      const dialog = page.getByRole("alertdialog");
      await expect(dialog).toBeVisible({ timeout: 30_000 });
      await dialog.getByRole("button", { name: "Allow", exact: true }).click();
      await expect(page.getByText("Đã ghi hello-e2e.txt sau khi bạn duyệt")).toBeVisible({
        timeout: 30_000,
      });
    });

    test("A3: viewport mobile — chat dùng được, không tràn ngang", async ({ page }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await loginAndOpenSession(page);

      const composer = page.getByLabel(COMPOSER);
      await expect(composer).toBeEnabled();
      await composer.fill("thử mobile");
      await page.getByRole("button", { name: "Send" }).click();
      await expect(page.getByText("Trả lời E2E cho viewport mobile")).toBeVisible({
        timeout: 30_000,
      });

      // Quy tắc một màn hình (AGENTS 0.11, 12.1): cửa sổ không bao giờ cuộn ngang —
      // trang phải bó trong viewport, nội dung dài chỉ cuộn bên trong khung con.
      const horizontalOverflow = await page.evaluate(() => {
        const el = document.scrollingElement;
        return el === null ? Number.POSITIVE_INFINITY : el.scrollWidth - el.clientWidth;
      });
      expect(horizontalOverflow).toBeLessThanOrEqual(0);
    });
  });
