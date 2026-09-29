import { useNavigate, useParams } from "react-router";

import { EmptyState, ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import {
  useApproveSkillDraft,
  useRejectSkillDraft,
  useSkill,
  useSkillDrafts,
  useSkills,
} from "@/features/skills/queries";
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
                  className={`w-full rounded-lg px-3 py-2 text-left transition ${name === skill.name ? "brand-wash font-semibold text-brand" : "hover:bg-accent"}`}
                >
                  <span className="block font-mono text-sm">{skill.name}</span>
                  <span className="mt-1 block text-xs text-muted-foreground">{skill.description}</span>
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
          <SkillDrafts />
        </div>
      </div>
    </PageShell>
  );
}

function SkillDrafts() {
  const { t } = useI18n();
  const drafts = useSkillDrafts();
  const approve = useApproveSkillDraft();
  const reject = useRejectSkillDraft();
  const busy = approve.isPending || reject.isPending;
  const mutationError = approve.error ?? reject.error;

  return (
    <Panel className="border-dashed">
      <h2 className="font-semibold">{t("skills.draftsTitle")}</h2>
      {drafts.isPending ? <LoadingState label={t("common.loading")} /> : null}
      {drafts.isError ? <ErrorState onRetry={() => void drafts.refetch()} /> : null}
      {mutationError ? (
        <p role="alert" className="mt-3 text-sm text-destructive">
          {t("skills.draftActionError")}
        </p>
      ) : null}
      {drafts.data?.drafts.length ? (
        <div role="status" className="mt-3 rounded-lg border border-need bg-need/10 p-3 text-sm text-need">
          {t("skills.draftsNewNotification")}
        </div>
      ) : null}
      {drafts.data?.drafts.length === 0 ? (
        <p className="mt-2 text-sm text-muted-foreground">{t("skills.draftsEmpty")}</p>
      ) : null}
      <div className="mt-4 space-y-5">
        {drafts.data?.drafts.map((draft) => (
          <article key={draft.id} className="rounded-lg border border-border p-4">
            <div className="flex flex-wrap items-start justify-between gap-2">
              <div>
                <h3 className="font-mono font-semibold">{draft.name}</h3>
                <p className="mt-1 text-xs uppercase tracking-wide text-muted-foreground">
                  {draft.kind === "update" ? t("skills.draftUpdate") : t("skills.draftNew")}
                </p>
              </div>
              <time className="text-xs text-muted-foreground" dateTime={draft.created_at}>
                {new Date(draft.created_at).toLocaleString()}
              </time>
            </div>
            <p className="mt-3 text-sm">{draft.description}</p>
            <p className="mt-2 text-xs text-muted-foreground">{draft.reason}</p>
            <pre className="mt-3 max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-card p-4 font-mono text-xs leading-5 text-foreground">
              {draft.content}
            </pre>
            <div className="mt-3 flex flex-wrap gap-2">
              <button
                type="button"
                disabled={busy}
                onClick={() => approve.mutate(draft.id)}
                className="rounded-lg bg-brand px-3 py-2 text-sm font-semibold text-primary-foreground disabled:cursor-not-allowed disabled:opacity-50"
              >
                {t("skills.draftApprove")}
              </button>
              <button
                type="button"
                disabled={busy}
                onClick={() => reject.mutate(draft.id)}
                className="rounded-lg border border-border px-3 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-50"
              >
                {t("skills.draftReject")}
              </button>
            </div>
          </article>
        ))}
      </div>
    </Panel>
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
            <p className="mt-1 text-sm text-muted-foreground">{skill.data.description}</p>
            <p className="mt-2 break-all text-xs text-muted-foreground">{skill.data.directory}</p>
          </div>
          <h3 className="mb-2 text-sm font-semibold">{t("skills.content")}</h3>
          <pre className="max-h-[32rem] overflow-auto whitespace-pre-wrap break-words rounded-lg bg-card p-4 font-mono text-xs leading-5 text-foreground">
            {skill.data.content}
          </pre>
        </>
      ) : null}
    </Panel>
  );
}
