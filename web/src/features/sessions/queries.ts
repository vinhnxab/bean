import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { CreateSessionRequest, UpdateSessionRequest } from "@/api/bindings";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export const sessionQueryKey = queryKeys.sessions;

export function useSessions(query: { q?: string; archived?: boolean } = {}) {
  const q = query.q?.trim() ?? "";
  const archived = query.archived ?? false;
  return useQuery({
    queryKey: queryKeys.sessionList(q, archived),
    queryFn: ({ signal }) => api.listSessions({ q, archived }, signal),
  });
}

export function useCreateSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: CreateSessionRequest) => api.createSession(request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionQueryKey }),
  });
}

export function useUpdateSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, request }: { id: number; request: UpdateSessionRequest }) =>
      api.updateSession(id, request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionQueryKey }),
  });
}

export function useDeleteSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => api.deleteSession(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: sessionQueryKey }),
  });
}
