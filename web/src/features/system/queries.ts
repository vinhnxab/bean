import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

/**
 * Cấu hình runtime đã khử secret.
 *
 * Không poll: `bean.toml` chỉ đổi khi Bean được khởi động lại (config nạp một lần),
 * nên một lần mỗi phiên là đúng — và `staleTime` dài tránh gọi lại vô ích khi người
 * dùng chuyển qua lại giữa các màn.
 */
export function useSystem() {
  return useQuery({
    queryKey: queryKeys.system,
    queryFn: ({ signal }) => api.system(signal),
    staleTime: 300_000,
  });
}
