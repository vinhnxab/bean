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
    if (file.data && draft === null) setDraft(file.data.content);
  }, [draft, file.data]);

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
    const contentToSave = draft ?? original;
    try {
      await save.mutateAsync({ name, request: { content: contentToSave } });
      setConfirmOpen(false);
      setDraft(null); // Reset to null so useEffect syncs with file.data
    } catch {
      // Mutation error is rendered below; keep the draft so the user can retry.
    }
  }

  const title = name === "MEMORY" ? t("memory.memoryFile") : t("memory.userFile");
  return (
    // Panel bó tối đa ~nửa màn hình (mục 12.1): nội dung dài thì textarea tự
    // cuộn bên trong, header và nút Lưu luôn đứng yên — hai editor + gap không
    // vượt quá vùng `main` có được, thân trang không dài ra theo dữ liệu.
    <Panel className="flex max-h-[calc(50svh_-_8rem)] flex-col">
      <div className="mb-3 flex shrink-0 flex-wrap items-center justify-between gap-2">
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
            className="min-h-0 w-full flex-1 resize-none rounded-lg border border-border bg-background p-3 font-mono text-sm outline-none focus:border-live focus:ring-2 focus:ring-live/30"
          />
          <div className="mt-3 flex shrink-0 justify-end">
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
    // `max-h` + `flex-col`: panel bó theo chiều cao màn hình thay vì phình theo
    // số ghi nhớ — thân trang (main) không phải dài ra theo dữ liệu. Danh sách
    // nằm trong vùng `min-h-0 flex-1 overflow-y-auto` để tự cuộn bên trong,
    // header và ô tìm kiếm luôn đứng yên. `xl:sticky` giữ panel tại chỗ khi
    // cuộn qua phần editor ở cột trái (layout hai cột). `px-2 pb-2` tạo khoảng
    // hở giữa card và scrollbar (kiểu pill macOS) để nội dung không dính sát.
    <Panel className="flex max-h-[calc(100svh_-_16rem)] flex-col xl:sticky xl:top-8">
      <div className="mb-4 flex shrink-0 flex-wrap items-center justify-between gap-3">
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
        className="mb-4 w-full shrink-0 rounded-lg border border-border bg-card px-3 py-2 text-sm outline-none focus:border-live"
      />
      {/* `@container`: số cột theo chiều rộng panel chứ không theo viewport —
          cùng một layout hai cột trên desktop, panel phải hẹp hơn cột trái nên
          giữ 1 cột; trên tablet panel chiếm trọn chiều rộng thì nới 2-3 cột. */}
      <div className="@container min-h-0 flex-1 overflow-y-auto px-2 pb-2">
        {memories.isPending ? <LoadingState label={t("common.loading")} /> : null}
        {memories.isError ? <ErrorState onRetry={() => void memories.refetch()} /> : null}
        {memories.data?.memories.length === 0 ? (
          <EmptyState title={t("memory.empty")} description={t("memory.emptyDescription")} />
        ) : null}
        {/* `content-start`: nếu thiếu, grid kéo giãn hàng auto cho khớp chiều
            cao panel làm card cao lửng; với content-start các hàng co theo nội dung. */}
        <ul className="grid grid-cols-1 content-start gap-3 @lg:grid-cols-2 @4xl:grid-cols-3">
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
      </div>
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
