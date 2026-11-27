import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import type {
  WorkNoteCreateRequest,
  WorkProjectCreateRequest,
  WorkProjectStatusRequest,
  WorkTaskCreateRequest,
  WorkTaskUpdateRequest,
} from "@/api/bindings";
import { api, type WorkTaskListQuery } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export function useWorkProjects() {
  return useQuery({
    queryKey: queryKeys.workProjects,
    queryFn: ({ signal }) => api.listWorkProjects(signal),
  });
}

export function useCreateWorkProject() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: WorkProjectCreateRequest) => api.createWorkProject(request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.work }),
  });
}

export function useUpdateWorkProject() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, request }: { id: number; request: WorkProjectStatusRequest }) =>
      api.updateWorkProject(id, request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.work }),
  });
}

export function useWorkTasks(filter: WorkTaskListQuery) {
  return useQuery({
    queryKey: queryKeys.workTasks(filter),
    queryFn: ({ signal }) => api.listWorkTasks(filter, signal),
  });
}

export function useCreateWorkTask() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: WorkTaskCreateRequest) => api.createWorkTask(request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.work }),
  });
}

/**
 * `patch` là bản đồ các trường muốn đổi — các trường còn lại gửi `null`
 * (server coi `null` là "giữ nguyên", khớp `WorkTaskUpdateRequest`).
 */
export type WorkTaskPatch = Partial<
  Pick<WorkTaskUpdateRequest, "title" | "notes" | "status" | "priority" | "assignee">
>;

export function useUpdateWorkTask() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, patch }: { id: number; patch: WorkTaskPatch }) =>
      api.updateWorkTask(id, {
        title: patch.title ?? null,
        notes: patch.notes ?? null,
        status: patch.status ?? null,
        priority: patch.priority ?? null,
        assignee: patch.assignee ?? null,
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.work }),
  });
}

export function useDeleteWorkTask() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => api.deleteWorkTask(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.work }),
  });
}

/** Bật khi bung ghi chú của task (`enabled`) — không tải notes của task đang gập. */
export function useWorkNotes(taskId: number, enabled: boolean) {
  return useQuery({
    queryKey: queryKeys.workNotes(taskId),
    queryFn: ({ signal }) => api.listWorkNotes(taskId, signal),
    enabled,
  });
}

export function useAddWorkNote() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ taskId, request }: { taskId: number; request: WorkNoteCreateRequest }) =>
      api.addWorkNote(taskId, request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.work }),
  });
}
