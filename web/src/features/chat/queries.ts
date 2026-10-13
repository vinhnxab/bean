import { useInfiniteQuery, useQuery } from "@tanstack/react-query";

import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export const MESSAGE_PAGE_SIZE = 30;

/** Lịch sử message chỉ đến từ REST; query key theo session để invalidate an toàn. */
export function useSessionMessages(sessionId: number) {
  return useInfiniteQuery({
    queryKey: queryKeys.messages(sessionId),
    queryFn: ({ pageParam, signal }) => api.listMessages(sessionId, pageParam, signal),
    initialPageParam: null as number | null,
    getNextPageParam: (lastPage) =>
      lastPage.messages.length === MESSAGE_PAGE_SIZE ? (lastPage.messages[0]?.seq ?? null) : undefined,
  });
}

/**
 * Trạng thái voice (M27). UI gọi một lần để quyết định có hiện nút mic.
 *
 * Cache 5 phút: voice chỉ đổi khi sửa `bean.toml` rồi khởi động lại — hỏi lại mỗi
 * lần render là lãng phí và làm nút mic nhấp nháy khi mạng chậm.
 */
export function useVoiceStatus() {
  return useQuery({
    queryKey: ["voice", "status"] as const,
    queryFn: ({ signal }) => api.voiceStatus(signal),
    staleTime: 5 * 60 * 1000,
  });
}
