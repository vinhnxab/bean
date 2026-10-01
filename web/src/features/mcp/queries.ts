import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

/**
 * MCP server đã khai báo + trạng thái discovery.
 *
 * Không poll: `connected` chỉ đổi khi Bean khởi động lại (MCP được nạp một lần lúc
 * boot), nên lấy một lần theo phiên là đủ.
 */
export function useMcpServers() {
  return useQuery({
    queryKey: queryKeys.mcp,
    queryFn: ({ signal }) => api.listMcpServers(signal),
    staleTime: 60_000,
  });
}

/** Đếm server đang nói chuyện được — widget Hub dùng con số này, không tự đếm lại. */
export function connectedCount(servers: { connected: boolean }[]): number {
  return servers.filter((server) => server.connected).length;
}
