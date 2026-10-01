import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

/** Số ngày mặc định của biểu đồ — khớp mặc định phía server (`DEFAULT_USAGE_DAYS`). */
export const USAGE_WINDOW_DAYS = 14;

/**
 * Token đã dùng theo từng ngày.
 *
 * `refetchInterval` dài (5 phút) vì đây là số liệu cả ngày: nó đổi chậm và người
 * dùng không ra quyết định theo phút. Nhánh nào cần số tức thời thì đọc
 * `/api/status` (`tokens_used`), widget này cho biết **xu hướng**, không phải nhịp.
 */
export function useUsage(days = USAGE_WINDOW_DAYS) {
  return useQuery({
    queryKey: queryKeys.usage(days),
    queryFn: ({ signal }) => api.usage(days, signal),
    refetchInterval: 300_000,
  });
}

/** Tổng token trong cửa sổ — tính một chỗ để widget và nhãn không lệch nhau. */
export function windowTotal(days: { total_tokens: number }[]): number {
  return days.reduce((sum, day) => sum + day.total_tokens, 0);
}
