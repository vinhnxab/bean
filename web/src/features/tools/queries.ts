import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

/**
 * Danh sách tool của agent.
 *
 * Không có `refetchInterval`: registry chỉ đổi khi Bean khởi động lại (hoặc khi
 * MCP server reconnect), nên tải một lần theo phiên là đủ — khác `/api/agents`,
 * thứ là trạng thái sống.
 */
export function useTools() {
  return useQuery({
    queryKey: queryKeys.tools,
    queryFn: ({ signal }) => api.listTools(signal),
    staleTime: 60_000,
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
