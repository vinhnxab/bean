import { useCallback, useEffect, useState } from "react";

/**
 * Trạng thái thu gọn của sidebar.
 *
 * Vì sao lưu vào `localStorage`
 *
 * Sidebar là thanh công cụ dùng cả ngày: người dùng mở rộng để đọc tên hội
 * thoại rồi thu gọn lại để lấy chỗ cho nội dung. Nếu mỗi lần tải trang đều trả
 * về mặc định thì việc thu gọn trở thành thao tác phải làm lại liên tục — đúng
 * cái mà `theme.tsx` đã làm với chủ đề, chỉ là cùng một bài toán.
 *
 * Chỉ `md:` trở lên mới thu gọn. Trên điện thoại sidebar là ngăn kéo phủ toàn
 * màn hình (`mobileOpen`); thu gọn nó thành một rail icon 80px sẽ biến thanh
 * điều hướng thành bảng ký hiệu không ai đọc được.
 */
export const SIDEBAR_COLLAPSED_KEY = "bean.sidebar.collapsed";

/** Mặc định mở rộng: màn đầu tiên sau khi đăng nhập cần nhìn thấy tên hội thoại. */
const DEFAULT_COLLAPSED = false;

/** Đọc lựa chọn đã lưu; `window` có thể không tồn tại (render/test ngoài trình duyệt). */
function readStored(): boolean {
  if (typeof window === "undefined") return DEFAULT_COLLAPSED;
  try {
    return window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "1";
  } catch {
    // Chế độ riêng tư / storage bị chặn — dùng mặc định.
    return DEFAULT_COLLAPSED;
  }
}

/** `[đang thu gọn, hàm bật/tắt]`. Lựa chọn được ghi lại mỗi lần đổi. */
export function useSidebarCollapsed(): [boolean, () => void] {
  // Khởi tạo lười (hàm khởi tạo) để lần render đầu đã đúng, không nhảy rail
  // từ rộng sang hẹp sau khi đọc `localStorage`.
  const [collapsed, setCollapsed] = useState<boolean>(readStored);

  useEffect(() => {
    try {
      window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, collapsed ? "1" : "0");
    } catch {
      // Storage bị chặn: vẫn bật/tắt được trong phiên hiện tại, chỉ mất lựa chọn
      // sau khi tải lại. Không đáng báo lỗi ra giao diện.
    }
  }, [collapsed]);

  const toggle = useCallback(() => setCollapsed((value) => !value), []);

  return [collapsed, toggle];
}
