import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { VoiceUpdateRequest } from "@/api/bindings";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export const MESSAGE_PAGE_SIZE = 30;

/** Lịch sử message chỉ đến từ REST; query key theo session để invalidate an toàn. */
export function useSessionMessages(sessionId: number) {
  return useInfiniteQuery({
    queryKey: queryKeys.messages(sessionId),
    queryFn: ({ pageParam, signal }) => api.listMessages(sessionId, pageParam, signal),
    initialPageParam: null as number | null,
    getNextPageParam: (lastPage) =>
      lastPage.messages.length === MESSAGE_PAGE_SIZE ? (lastPage.messages[0]?.seq ?? null) : undefined,
  });
}

/** Query key trạng thái voice — dùng chung cho query và mutation (set cache sau PATCH). */
export const VOICE_STATUS_QUERY_KEY = ["voice", "status"] as const;

/**
 * Trạng thái voice (M27). UI gọi một lần để dựng nút mic và form cấu hình.
 *
 * Cache 5 phút: voice chỉ đổi qua `PATCH /api/voice` — hỏi lại mỗi lần render là
 * lãng phí và làm nút mic nhấp nháy khi mạng chậm. Lần duy nhất cache được cập
 * nhật trực tiếp là sau `useUpdateVoice` (server trả snapshot mới nhất).
 */
export function useVoiceStatus() {
  return useQuery({
    queryKey: VOICE_STATUS_QUERY_KEY,
    queryFn: ({ signal }) => api.voiceStatus(signal),
    staleTime: 5 * 60 * 1000,
  });
}

/**
 * Đổi voice lúc chạy (chỉ admin): bật/tắt **và** provider/ngôn ngữ/API key. Server
 * persist rồi trả snapshot mới; ghi thẳng vào cache thay vì invalidate để nút mic
 * và form phản ánh ngay cả khi staleTime chưa hết.
 */
export function useUpdateVoice() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: VoiceUpdateRequest) => api.updateVoice(request),
    onSuccess: (updated) => {
      queryClient.setQueryData(VOICE_STATUS_QUERY_KEY, updated);
    },
  });
}
