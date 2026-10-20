import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  LlmModelUpdateRequest,
  LlmProviderCreateRequest,
  LlmProviderSwitchRequest,
  LlmProviderUpdateRequest,
} from "@/api/bindings";
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

/**
 * Hot-swap provider theo profile (`[[llm.providers]]` hoặc "default").
 *
 * Server trả nguyên snapshot `LlmConfigResponse` sau khi switch — ghi đè cache
 * một lần (provider, model, allowed_models, danh sách profile cùng đổi). Cũng vô
 * hiệu `status`/`system` vì model hiện tại đã đổi.
 */
export function useSwitchLlmProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: LlmProviderSwitchRequest) => api.switchLlmProvider(request),
    onSuccess: (snapshot) => {
      queryClient.setQueryData(queryKeys.llmConfig, snapshot);
      void queryClient.invalidateQueries({ queryKey: queryKeys.status });
      void queryClient.invalidateQueries({ queryKey: queryKeys.system });
    },
  });
}

/**
 * Thêm profile provider mới (ghi vào `[[llm.providers]]` của bean.toml).
 *
 * Server trả snapshot sau khi thêm — ghi đè cache `llmConfig` một lần thay vì
 * refetch, vì danh sách profile vừa đổi phía server.
 */
export function useCreateLlmProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: LlmProviderCreateRequest) => api.createLlmProvider(request),
    onSuccess: (snapshot) => {
      queryClient.setQueryData(queryKeys.llmConfig, snapshot);
    },
  });
}

/** Sửa profile (tên là định danh, không đổi). Xem `useCreateLlmProvider` về cache. */
export function useUpdateLlmProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, request }: { name: string; request: LlmProviderUpdateRequest }) =>
      api.updateLlmProviderProfile(name, request),
    onSuccess: (snapshot) => {
      queryClient.setQueryData(queryKeys.llmConfig, snapshot);
      // Profile active có thể vừa đổi model → hai query hiển thị model cần làm mới.
      void queryClient.invalidateQueries({ queryKey: queryKeys.status });
      void queryClient.invalidateQueries({ queryKey: queryKeys.system });
    },
  });
}

/** Xoá profile; server chặn 409 nếu profile đang active. Xem `useCreateLlmProvider`. */
export function useDeleteLlmProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => api.deleteLlmProviderProfile(name),
    onSuccess: (snapshot) => {
      queryClient.setQueryData(queryKeys.llmConfig, snapshot);
    },
  });
}
