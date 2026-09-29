import { useEffect, useState } from "react";

import type { MemoryDto } from "@/api/bindings";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { EmptyState, ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import {
  type MemoryFileName,
  useDeleteMemory,
  useMemories,
  useMemoryFile,
  useSaveMemoryFile,
} from "@/features/memory/queries";
import { useI18n } from "@/i18n";
import { formatDateTime } from "@/lib/format";

export function MemoryPage() {
  const { t } = useI18n();
  return (
    <PageShell title={t("memory.title")} description={t("memory.description")}>
      <div className="grid gap-6 xl:grid-cols-[minmax(0,1.2fr)_minmax(20rem,0.8fr)]">
        <div className="space-y-6">
          <MemoryFileEditor name="MEMORY" />
          <MemoryFileEditor name="USER" />
        </div>
        <MemoryList />
      </div>
    </PageShell>
  );
}

function MemoryFileEditor({ name }: { name: MemoryFileName }) {
  const { t } = useI18n();
  const file = useMemoryFile(name);
  const save = useSaveMemoryFile();
  const [draft, setDraft] = useState<string | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const original = file.data?.content ?? "";
  const dirty = draft !== null && draft !== original;

  useEffect(() => {
    if (file.data && (draft === null || !dirty)) setDraft(file.data.content);
  }, [dirty, draft, file.data]);

  useEffect(() => {
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);

  async function confirmSave() {
    try {
      await save.mutateAsync({ name, request: { content: draft ?? original } });
      setConfirmOpen(false);
    } catch {
      // Mutation error is rendered below; keep the draft so the user can retry.
    }
  }

  const title = name === "MEMORY" ? t("memory.memoryFile") : t("memory.userFile");
  return (
    <Panel>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="font-semibold">{title}</h2>
          <p className="text-xs text-muted-foreground">{name}.md</p>
        </div>
        <span className={`text-xs font-semibold ${dirty ? "text-need" : "text-live"}`} role="status">
          {dirty ? t("memory.unsaved") : t("memory.saved")}
        </span>
      </div>
      {file.isPending ? <LoadingState label={t("common.loading")} /> : null}
      {file.isError ? <ErrorState onRetry={() => void file.refetch()} /> : null}
      {file.data ? (
        <>
          <label className="sr-only" htmlFor={`memory-file-${name}`}>
            {title}
          </label>
          <textarea
            id={`memory-file-${name}`}
            value={draft ?? original}
            onChange={(event) => setDraft(event.target.value)}
            rows={12}
            spellCheck={false}
            className="w-full resize-y rounded-lg border border-border bg-background p-3 font-mono text-sm outline-none focus:border-live focus:ring-2 focus:ring-live/30"
          />
          <div className="mt-3 flex justify-end">
            <button
              type="button"
              disabled={!dirty || save.isPending}
              onClick={() => setConfirmOpen(true)}
              className="rounded-lg bg-brand px-3 py-2 text-sm font-semibold text-brand-ink hover:bg-brand-strong disabled:opacity-50"
            >
              {save.isPending ? t("common.loading") : t("memory.save")}
            </button>
          </div>
        </>
      ) : null}
      {save.isError ? (
        <p className="mt-2 text-sm text-destructive" role="alert">
          {t("memory.saveError")}
        </p>
      ) : null}
      <ConfirmDialog
        open={confirmOpen}
        title={t("memory.confirmSaveTitle")}
        message={t("memory.confirmSaveMessage")}
        confirmLabel={t("memory.save")}
        cancelLabel={t("common.cancel")}
        pending={save.isPending}
        onConfirm={() => void confirmSave()}
        onCancel={() => setConfirmOpen(false)}
      />
    </Panel>
  );
}

function MemoryList() {
  const { t } = useI18n();
  const [search, setSearch] = useState("");
  const memories = useMemories(search);
  const remove = useDeleteMemory();
  const [target, setTarget] = useState<MemoryDto | null>(null);

  async function confirmDelete() {
    if (!target) return;
    try {
      await remove.mutateAsync(target.id);
      setTarget(null);
    } catch {
      // Keep the dialog open when the server rejects the delete.
    }
  }

  return (
    <Panel>
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="font-semibold">{t("memory.listTitle")}</h2>
          <p className="text-xs text-muted-foreground">{t("memory.listDescription")}</p>
        </div>
      </div>
      <label className="sr-only" htmlFor="memory-search">
        {t("memory.search")}
      </label>
      <input
        id="memory-search"
        type="search"
        value={search}
        onChange={(event) => setSearch(event.target.value)}
        placeholder={t("memory.searchPlaceholder")}
        className="mb-4 w-full rounded-lg border border-border bg-card px-3 py-2 text-sm outline-none focus:border-live"
      />
      {memories.isPending ? <LoadingState label={t("common.loading")} /> : null}
      {memories.isError ? <ErrorState onRetry={() => void memories.refetch()} /> : null}
      {memories.data?.memories.length === 0 ? (
        <EmptyState title={t("memory.empty")} description={t("memory.emptyDescription")} />
      ) : null}
      <ul className="space-y-3">
        {memories.data?.memories.map((memory) => (
          <li key={memory.id} className="rounded-lg border border-border p-3">
            <div className="flex items-start justify-between gap-3">
              <p className="whitespace-pre-wrap break-words text-sm">{memory.text}</p>
              <button
                type="button"
                onClick={() => setTarget(memory)}
                className="shrink-0 text-xs font-semibold text-destructive hover:underline"
              >
                {t("common.delete")}
              </button>
            </div>
            <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
              {memory.tags ? <span>#{memory.tags}</span> : null}
              <time dateTime={memory.created_at}>{formatDateTime(memory.created_at)}</time>
            </div>
          </li>
        ))}
      </ul>
      <ConfirmDialog
        open={target !== null}
        title={t("memory.deleteTitle")}
        message={target ? `${t("memory.deleteMessage")}\n${target.text}` : ""}
        confirmLabel={t("common.delete")}
        cancelLabel={t("common.cancel")}
        danger
        pending={remove.isPending}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setTarget(null)}
      />
    </Panel>
  );
}
