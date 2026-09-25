import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { CreateSessionRequest, SessionDto, SessionListResponse } from "@/api/bindings";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export const sessionQueryKey = queryKeys.sessions;

export function useSessions() {
  return useQuery({
    queryKey: sessionQueryKey,
    queryFn: ({ signal }) => api.listSessions(signal),
  });
}

export function useCreateSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: CreateSessionRequest) => api.createSession(request),
    onSuccess: (created: SessionDto) => {
      queryClient.setQueryData<SessionListResponse>(sessionQueryKey, (previous) => ({
        sessions: [created, ...(previous?.sessions.filter((item) => item.id !== created.id) ?? [])],
      }));
    },
  });
}
