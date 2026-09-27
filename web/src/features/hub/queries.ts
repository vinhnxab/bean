import { useQuery } from "@tanstack/react-query";
import type { AgentReportDto } from "@/api/bindings";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

/**
 * Báo cáo agent cho HUB.
 *
 * `refetchInterval` ngắn vì trạng thái sống là thứ người dùng cần thấy ngay khi
 * mở app; nhưng còn vừa phải vì đây là bảng điều khiển, không phải bảng giá.
 * Sự kiệm WebSocket (`Sync`, `Final`) sẽ invalidate cache để phản ứng tức thì.
 */
export function useAgents() {
  return useQuery({
    queryKey: queryKeys.agents,
    queryFn: ({ signal }) => api.listAgents(signal),
    refetchInterval: 15_000,
  });
}

/** Trạng thái sống → nhãn hiển thị; dùng chung để sơ đồ và danh sách không lệch. */
export function statusTone(status: AgentReportDto["status"]) {
  switch (status) {
    case "working":
      return "working" as const;
    case "awaiting_you":
      return "awaiting" as const;
    default:
      return "idle" as const;
  }
}
