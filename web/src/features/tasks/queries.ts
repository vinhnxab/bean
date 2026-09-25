import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { TaskRequest, TaskUpdateRequest } from "@/api/bindings";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export function useTasks() {
  return useQuery({
    queryKey: queryKeys.tasks,
    queryFn: ({ signal }) => api.listTasks(signal),
  });
}

export function useCreateTask() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: TaskRequest) => api.createTask(request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.tasks }),
  });
}

export function useUpdateTask() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, request }: { id: number; request: TaskUpdateRequest }) => api.updateTask(id, request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.tasks }),
  });
}

export function useDeleteTask() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => api.deleteTask(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.tasks }),
  });
}
