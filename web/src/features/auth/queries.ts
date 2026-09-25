import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { AuthMeResponse, LoginRequest } from "@/api/bindings";
import { api } from "@/api/client";

export type AuthUser = AuthMeResponse;

export const queryKeys = {
  auth: ["auth", "me"] as const,
  sessions: ["sessions", { archived: false }] as const,
  messages: (sessionId: number) => ["messages", sessionId] as const,
};

export function useAuth() {
  return useQuery({
    queryKey: queryKeys.auth,
    queryFn: ({ signal }) => api.me(signal),
    retry: false,
    staleTime: 30_000,
  });
}

export function useLogin() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: LoginRequest) => api.login(request),
    onSuccess: (user) => queryClient.setQueryData(queryKeys.auth, user),
  });
}

export function useLogout() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => api.logout(),
    onSuccess: () => queryClient.clear(),
  });
}
