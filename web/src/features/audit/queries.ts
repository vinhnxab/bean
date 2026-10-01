import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export const AUDIT_PAGE_SIZE = 25;

export function useAuditLog() {
  return useInfiniteQuery({
    queryKey: queryKeys.audit,
    initialPageParam: null as number | null,
    queryFn: ({ pageParam, signal }) => api.listAudit(pageParam, signal),
    getNextPageParam: (lastPage, pages) =>
      lastPage.entries.length === AUDIT_PAGE_SIZE ? pages.length * AUDIT_PAGE_SIZE : undefined,
  });
}

/** Số dòng widget HUB hiện — khớp `slice` trong `ActivityFeed`. */
export const AUDIT_FEED_SIZE = 5;

/**
 * Năm dòng audit mới nhất cho widget HUB.
 *
 * Phân trang vô hạn là thừa ở đây: widget chỉ hiện 5 dòng và có nút "xem tất
 * cả" dẫn sang màn Audit. Nó cần khoá riêng — xem giải thích ở
 * `queryKeys.auditFeed`.
 */
export function useAuditFeed(limit = AUDIT_FEED_SIZE) {
  return useQuery({
    queryKey: [...queryKeys.auditFeed, limit],
    queryFn: ({ signal }) => api.listAudit(null, signal, limit),
    refetchInterval: 30_000,
  });
}
