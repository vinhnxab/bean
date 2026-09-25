import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { AuthMeResponse, LoginRequest } from "@/api/bindings";
import { api } from "@/api/client";

export type AuthUser = AuthMeResponse;

export const queryKeys = {
  auth: ["auth", "me"] as const,
  /** Prefix dùng để invalidate mọi biến thể danh sách session. */
  sessions: ["sessions"] as const,
  sessionList: (q: string, archived: boolean) => ["sessions", { q, archived }] as const,
  messages: (sessionId: number) => ["messages", sessionId] as const,
  memoryFiles: ["memory-files"] as const,
  memoryFile: (name: string) => ["memory-files", name] as const,
  memories: (q: string) => ["memories", { q }] as const,
  skills: ["skills"] as const,
  skill: (name: string) => ["skills", name] as const,
  skillDrafts: ["skills", "drafts"] as const,
  tasks: ["tasks"] as const,
  audit: ["audit"] as const,
  status: ["status"] as const,
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
