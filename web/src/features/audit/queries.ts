import { useInfiniteQuery } from "@tanstack/react-query";
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
