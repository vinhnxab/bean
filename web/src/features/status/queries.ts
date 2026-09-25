import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export function useStatus() {
  return useQuery({
    queryKey: queryKeys.status,
    queryFn: ({ signal }) => api.status(signal),
    refetchInterval: 30_000,
  });
}
