import { useInfiniteQuery } from "@tanstack/react-query";

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
