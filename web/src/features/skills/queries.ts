import { useQuery } from "@tanstack/react-query";
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
