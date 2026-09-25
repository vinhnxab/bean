import { useNavigate, useParams } from "react-router";

import { EmptyState, ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import { useSkill, useSkills } from "@/features/skills/queries";
import { useI18n } from "@/i18n";

export function SkillsPage() {
  const { t } = useI18n();
  const { name } = useParams();
  const skills = useSkills();
  const navigate = useNavigate();

  return (
    <PageShell title={t("skills.title")} description={t("skills.description")}>
      <div className="grid gap-6 lg:grid-cols-[minmax(16rem,0.7fr)_minmax(0,1.3fr)]">
        <Panel>
          <h2 className="mb-3 font-semibold">{t("skills.listTitle")}</h2>
          {skills.isPending ? <LoadingState label={t("common.loading")} /> : null}
          {skills.isError ? <ErrorState onRetry={() => void skills.refetch()} /> : null}
          {skills.data?.skills.length === 0 ? (
            <EmptyState title={t("skills.empty")} description={t("skills.emptyDescription")} />
          ) : null}
          <ul className="space-y-1">
            {skills.data?.skills.map((skill) => (
              <li key={skill.name}>
                <button
                  type="button"
                  onClick={() => navigate(`/skills/${encodeURIComponent(skill.name)}`)}
                  className={`w-full rounded-xl px-3 py-2 text-left transition ${name === skill.name ? "bg-emerald-100 font-semibold text-emerald-900 dark:bg-emerald-950 dark:text-emerald-200" : "hover:bg-slate-100 dark:hover:bg-slate-800"}`}
                >
                  <span className="block font-mono text-sm">{skill.name}</span>
                  <span className="mt-1 block text-xs text-slate-500 dark:text-slate-400">
                    {skill.description}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </Panel>
        <div className="space-y-6">
          {name ? (
            <SkillDetail name={name} />
          ) : (
            <EmptyState title={t("skills.selectTitle")} description={t("skills.selectDescription")} />
          )}
          <Panel className="border-dashed">
            <h2 className="font-semibold">{t("skills.draftsTitle")}</h2>
            <p className="mt-2 text-sm text-slate-500 dark:text-slate-400">{t("skills.draftsEmpty")}</p>
            <p className="mt-1 text-xs text-slate-400 dark:text-slate-500">{t("skills.draftsM15")}</p>
          </Panel>
        </div>
      </div>
    </PageShell>
  );
}

function SkillDetail({ name }: { name: string }) {
  const { t } = useI18n();
  const skill = useSkill(name);
  return (
    <Panel>
      {skill.isPending ? <LoadingState label={t("common.loading")} /> : null}
      {skill.isError ? <ErrorState onRetry={() => void skill.refetch()} /> : null}
      {skill.data ? (
        <>
          <div className="mb-4">
            <h2 className="font-mono text-lg font-bold">{skill.data.name}</h2>
            <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">{skill.data.description}</p>
            <p className="mt-2 break-all text-xs text-slate-400 dark:text-slate-500">
              {skill.data.directory}
            </p>
          </div>
          <h3 className="mb-2 text-sm font-semibold">{t("skills.content")}</h3>
          <pre className="max-h-[32rem] overflow-auto whitespace-pre-wrap break-words rounded-xl bg-slate-950 p-4 font-mono text-xs leading-5 text-slate-100">
            {skill.data.content}
          </pre>
        </>
      ) : null}
    </Panel>
  );
}
