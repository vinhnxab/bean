import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

/**
 * Danh sách tool của agent.
 *
 * Registry vẫn chỉ đổi khi Bean khởi động lại (hoặc MCP server reconnect), nhưng
 * cờ `enabled` từng tool đổi được **tại runtime** qua toggle — mutation tương ứng
 * tự invalidate cache này ngay khi server xác nhận.
 */
export function useTools() {
  return useQuery({
    queryKey: queryKeys.tools,
    queryFn: ({ signal }) => api.listTools(signal),
    staleTime: 60_000,
  });
}

/**
 * Bật/tắt một tool (chỉ admin). Server trả trạng thái mới sau khi đã persist DB
 * **và** mutate registry; không có optimistic update — switch phải phản ánh đúng
 * trạng thái server đã xác nhận, không phải trạng thái client đoán trước.
 */
export function useSetToolEnabled() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, enabled }: { name: string; enabled: boolean }) => api.setToolEnabled(name, enabled),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.tools });
    },
  });
}

/** Nhãn rủi ro → tone của `Badge`; tone mang cả hình khối, không chỉ màu. */
export function riskTone(risk: "safe" | "confirm" | "dangerous") {
  switch (risk) {
    case "dangerous":
      return "danger" as const;
    case "confirm":
      return "awaiting" as const;
    default:
      return "idle" as const;
  }
}
