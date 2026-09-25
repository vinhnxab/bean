import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { SkillDraftListResponse } from "@/api/bindings";
import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";

export function useSkills() {
  return useQuery({
    queryKey: queryKeys.skills,
    queryFn: ({ signal }) => api.listSkills(signal),
  });
}

export function useSkill(name: string | null) {
  return useQuery({
    queryKey: queryKeys.skill(name ?? ""),
    queryFn: ({ signal }) => api.getSkill(name ?? "", signal),
    enabled: name !== null,
  });
}

export function useSkillDrafts() {
  return useQuery({
    queryKey: queryKeys.skillDrafts,
    queryFn: ({ signal }) => api.listSkillDrafts(signal),
  });
}

function useDraftDecision(approve: boolean) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => (approve ? api.approveSkillDraft(id, {}) : api.rejectSkillDraft(id, {})),
    onSuccess: (_response, id) => {
      queryClient.setQueryData<SkillDraftListResponse>(queryKeys.skillDrafts, (current) => ({
        drafts: current?.drafts.filter((draft) => draft.id !== id) ?? [],
      }));
      if (approve) void queryClient.invalidateQueries({ queryKey: queryKeys.skills });
    },
  });
}

export function useApproveSkillDraft() {
  return useDraftDecision(true);
}

export function useRejectSkillDraft() {
  return useDraftDecision(false);
}
