import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { LlmModelUpdateRequest } from "@/api/bindings";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

/** Cấu hình LLM runtime — cùng nguồn sự thật với lệnh `/model`. */
export function useLlmConfig() {
  return useQuery({
    queryKey: queryKeys.llmConfig,
    queryFn: ({ signal }) => api.llmConfig(signal),
  });
}

/**
 * Đổi model runtime.
 *
 * Lưu thành công phải cập nhật cache **và** vô hiệu `status`/`system`: hai query
 * đó hiện model cũ vì React Query không biết model vừa đổi trên server.
 */
export function useUpdateLlmModel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: LlmModelUpdateRequest) => api.updateLlmModel(request),
    onSuccess: (updated) => {
      queryClient.setQueryData(queryKeys.llmConfig, (current: Awaited<ReturnType<typeof api.llmConfig>>) =>
        current ? { ...current, model: updated.model } : current,
      );
      void queryClient.invalidateQueries({ queryKey: queryKeys.status });
      void queryClient.invalidateQueries({ queryKey: queryKeys.system });
    },
  });
}
