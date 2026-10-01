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
  /**
   * Nguồn cấp 5 dòng audit đầu cho widget HUB. Tách khoá khỏi `audit` vì hai
   * chỗ dùng hai kiểu query khác nhau trên cùng endpoint: màn Audit cần phân
   * trang vô hạn, HUB chỉ cần một trang đầu. Dùng chung khoá sẽ khiến React
   * Query phải chọn giữa `InfiniteData` và mảng phẳng — nguồn của lỗi dữ liệu
   * kiểu "lần nào cũng rỗng ở trang 2".
   */
  auditFeed: ["audit", "feed"] as const,
  status: ["status"] as const,
  /** Báo cáo agent đã lọc RBAC ở server — không lọc lại ở client. */
  agents: ["agents"] as const,
  /** Danh sách tool (cũng đã lọc RBAC ở server). */
  tools: ["tools"] as const,
  /** MCP server đã khai báo + trạng thái suy ra từ registry. */
  mcp: ["mcp"] as const,
  /** Token theo ngày — `days` là một phần của khoá vì đáp án khác nhau theo nó. */
  usage: (days: number) => ["usage", { days }] as const,
  /** Cấu hình runtime đã khử secret. */
  system: ["system"] as const,
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
