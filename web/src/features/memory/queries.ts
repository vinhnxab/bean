import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { MemoryFileRequest } from "@/api/bindings";
import { ApiRequestError, api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export type MemoryFileName = "MEMORY" | "USER";

export function useMemoryFile(name: MemoryFileName) {
  return useQuery({
    queryKey: queryKeys.memoryFile(name),
    queryFn: async ({ signal }) => {
      try {
        return await api.getMemoryFile(name, signal);
      } catch (error) {
        if (error instanceof ApiRequestError && error.status === 404) {
          return { name, content: "" };
        }
        throw error;
      }
    },
  });
}

export function useSaveMemoryFile() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, request }: { name: MemoryFileName; request: MemoryFileRequest }) =>
      api.putMemoryFile(name, request),
    onSuccess: (data, variables) => {
      queryClient.setQueryData(queryKeys.memoryFile(variables.name), data);
      void queryClient.invalidateQueries({ queryKey: queryKeys.memoryFiles });
    },
  });
}

export function useMemories(query: string) {
  const normalized = query.trim();
  return useQuery({
    queryKey: queryKeys.memories(normalized),
    queryFn: ({ signal }) => api.listMemories(normalized, signal),
  });
}

export function useDeleteMemory() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => api.deleteMemory(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["memories"] }),
  });
}
