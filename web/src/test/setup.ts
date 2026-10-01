// Thiết lập cho mọi test của Vitest (agents.md mục 3.2, 20).
// - jest-dom: matcher như `toBeInTheDocument()`, `toHaveTextContent()`.
// - KHÔNG bật `dangerouslySetInnerHTML` hay tài nguyên ngoài trong test (mục 0.9).
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, vi } from "vitest";

import { testServer } from "@/test/server";

/**
 * jsdom không có `navigator.clipboard`, mà component sao chép gọi
 * `navigator.clipboard?.writeText(...)`. Nếu không mock, lệnh đó rơi vào
 * `undefined` và test không phân biệt được "đã sao chép" với "hỏng".
 *
 * Dùng `stubGlobal` + khôi phục bằng `unstubAllGlobals` để không rò sang test
 * khác trong cùng file.
 */
beforeAll(() => {
  vi.stubGlobal("navigator", {
    ...navigator,
    clipboard: { writeText: vi.fn().mockResolvedValue(undefined) },
  });
});

/**
 * jsdom không cài `Element.prototype.scrollIntoView`, còn `@radix-ui/react-select`
 * gọi nó ở effect mount để cuộn mục đang chọn vào khung nhìn. Thiếu hàm này thì
 * `Select` **ném lỗi ngay khi mở**: test báo `Unable to find role="option"` chứ
 * không phải lý do thật, dễ khiến người đọc đi tìm ở component.
 *
 * Polyfill là hàm rỗng: cuộn không có ý nghĩa trong jsdom, nhưng việc nó **được
 * gọi** thì có — đó mới là thứ test đang cần khẳng định. Ghi lại ở đây thay vì
 * `vi.fn()` để không sinh thêm thứ cần dọn dẹp.
 */
beforeAll(() => {
  Element.prototype.scrollIntoView = function scrollIntoView() {};
});

/**
 * Radix (dropdown, dialog) khi đóng một lớp phủ sẽ đặt
 * `document.body { pointer-events: none }` rồi **chỉ gỡ khi lớp phủ mở lại**.
 * Nếu test kết thúc đúng lúc menu đang đóng, thuộc tính này còn sót lại trên
 * `body` của jsdom — và mọi `user.click` ở test kế tiếp trong cùng file sẽ bị
 * chặn. Triệu chứng rất đáng ngờ: test chạy đơn lẻ thì xanh, chạy cả file thì
 * fail, và lỗi là "không tìm thấy phần tử" chứ không phải "bị chặn con trỏ".
 *
 * Dọn nó ở mọi test: đây là trạng thái do thư viện để lại, không phải trạng
 * thái mà test nào cố ý dựng, nên xoá là đúng.
 */
afterEach(() => {
  cleanup();
  document.body.style.removeProperty("pointer-events");
  testServer.resetHandlers();
});

afterAll(() => {
  vi.unstubAllGlobals();
  testServer.close();
});

beforeAll(() => testServer.listen({ onUnhandledRequest: "error" }));
