import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  ChangePasswordRequest,
  LlmModelUpdateRequest,
  LlmProviderCreateRequest,
  LlmProviderSwitchRequest,
  LlmProviderUpdateRequest,
  SecretSlotDto,
  SettingDto,
  SettingsPatchRequest,
} from "@/api/bindings";
import { api, type SecretPutRequestPayload } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export type { SecretSlotDto, SettingDto };

export interface SecretSlotWithErrors {
  slot: SecretSlotDto;
  status: "idle" | "error";
  error: string | null;
}

/** Cấu hình LLM runtime — cùng nguồn sự thật với lệnh `/model`. */
export function useLlmConfig() {
  return useQuery({
    queryKey: queryKeys.llmConfig,
    queryFn: ({ signal }) => api.llmConfig(signal),
  });
}

/** Bảng thiết lập (M29) — giá trị hiệu lực + nguồn, không secret. */
export function useSettings() {
  return useQuery({
    queryKey: queryKeys.settings,
    queryFn: ({ signal }) => api.settings(signal),
  });
}

/** Slot secret — chỉ cờ `set`/`source`, không giá trị. */
export function useSecrets() {
  return useQuery({
    queryKey: queryKeys.secrets,
    queryFn: ({ signal }) => api.secrets(signal),
  });
}

/** Ghi một khoá thiết lập (chỉ tầng A). */
export function usePatchSetting() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: SettingsPatchRequest) => api.patchSetting(request),
    onMutate: () => {
      // Sửa cấu hình có thể làm thay đổi toàn bộ bảng (ví dụ source) — vô hiệu
      // query để UI đọc snapshot mới sau khi lưu.
      return queryClient.cancelQueries({ queryKey: queryKeys.settings });
    },
    onSuccess: (request) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.settings });
      if (request.key.endsWith("_env") || request.key.endsWith("_TOKEN")) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.secrets });
      }
    },
  });
}

/** Lưu secret vào `secrets.env` — step-up đã xảy ra ở server, client chỉ gửi `{value, password}`. */
export function usePutSecret() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, payload }: { name: string; payload: SecretPutRequestPayload }) =>
      api.putSecret(name, payload),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.secrets });
    },
  });
}

/** Xoá secret khỏi `secrets.env`. */
export function useDeleteSecret() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, password }: { name: string; password: string }) => api.deleteSecret(name, password),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.secrets });
    },
  });
}

/** Kiểm tra kết nối `llm` | `voice` | `telegram` | `search` — probe dùng chung với doctor. */
export function useTestConnection() {
  return useMutation({
    mutationFn: (target: string) => api.testConnection(target),
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

/** Thêm profile provider mới (ghi vào `[[llm.providers]]` của bean.toml). */
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

/**
 * Đổi mật khẩu.
 *
 * Sau khi thành công, server đã xoá cookie (Set-Cookie: Max-Age=0) và thu hồi
 * toàn bộ session. Invalidate query `auth` ngay để `useAuth` refetch — lần
 * refetch đó trả 401 → `unauthorizedHandler` trong `ProtectedRoute` navigate
 * về `/login`. Không dùng `setTimeout` + `clear()` vì `staleTime: 30_000` của
 * `useAuth` sẽ giữ cache cũ và không trigger redirect.
 */
export function useChangePassword() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: ChangePasswordRequest) => api.changePassword(request),
    onSuccess: () => {
      // Delay ngắn để user đọc thông báo thành công, rồi invalidate auth cache.
      // useAuth refetch → 401 (cookie đã bị server xoá) → unauthorizedHandler
      // trong ProtectedRoute navigate về /login.
      setTimeout(() => {
        void queryClient.invalidateQueries({ queryKey: queryKeys.auth });
      }, 1500);
    },
  });
}

/** Cập nhật trạng thái voice lúc chạy (bật/tắt, provider, ngôn ngữ, key…). */
export function useUpdateVoice() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: Parameters<typeof api.updateVoice>[0]) => api.updateVoice(request),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.llmConfig });
      void queryClient.invalidateQueries({ queryKey: queryKeys.status });
      void queryClient.invalidateQueries({ queryKey: queryKeys.system });
    },
  });
}
