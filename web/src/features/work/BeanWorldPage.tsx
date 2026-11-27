import { type FormEvent, useState } from "react";

import type { WorkProjectDto, WorkTaskDto } from "@/api/bindings";
import { HelpHint } from "@/components/HelpHint";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { EmptyState, ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import {
  useAddWorkNote,
  useCreateWorkProject,
  useCreateWorkTask,
  useDeleteWorkTask,
  useUpdateWorkProject,
  useUpdateWorkTask,
  useWorkNotes,
  useWorkProjects,
  useWorkTasks,
} from "@/features/work/queries";
import { useI18n } from "@/i18n";
import type { MessageKey } from "@/i18n/vi";
import { formatDateTime } from "@/lib/format";

/** Role subagent có thể giao việc — khớp các role trong `[[roles]]` của config. */
const ASSIGNEE_ROLES = [
  "developer",
  "qa",
  "marketing",
  "monitor",
  "security-scan",
  "finance-readonly",
] as const;

const TASK_STATUSES = ["open", "in_progress", "blocked", "done"] as const;

const PRIORITIES = ["low", "normal", "high"] as const;

type TaskStatus = (typeof TASK_STATUSES)[number];
type TaskPriority = (typeof PRIORITIES)[number];

/**
 * DTO trả `status`/`priority` là `string` còn `t()` cần khoá literal — hàm này
 * thu hẹp có kiểm tra runtime: server validate chặt hai trường này nên giá trị
 * luôn nằm trong tập; nếu lệch (server mới/cũ) thì trả `null` để hiển thị thô
 * thay vì sập trang.
 */
function statusKey(status: string): MessageKey | null {
  return TASK_STATUSES.includes(status as TaskStatus) ? `work.status.${status as TaskStatus}` : null;
}

function priorityKey(priority: string): MessageKey | null {
  return PRIORITIES.includes(priority as TaskPriority) ? `work.priority.${priority as TaskPriority}` : null;
}

export function BeanWorldPage() {
  const { t } = useI18n();
  const projects = useWorkProjects();

  return (
    <PageShell title={t("work.title")} description={t("work.description")}>
      <div className="grid gap-6 xl:grid-cols-[minmax(18rem,0.8fr)_minmax(0,1.2fr)]">
        <ProjectsPanel projects={projects.data?.projects ?? []} state={projects} />
        <TasksPanel projects={projects.data?.projects ?? []} />
      </div>
    </PageShell>
  );
}

// ---------------------------------------------------------------------------
// Projects — cột trái: tạo project (duyệt của người thật) + archive/reactivate
// ---------------------------------------------------------------------------

function ProjectsPanel({
  projects,
  state,
}: {
  projects: WorkProjectDto[];
  state: ReturnType<typeof useWorkProjects>;
}) {
  const { t } = useI18n();
  const create = useCreateWorkProject();
  const update = useUpdateWorkProject();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [formError, setFormError] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setFormError(false);
    if (!name.trim()) {
      setFormError(true);
      return;
    }
    try {
      await create.mutateAsync({ name: name.trim(), description: description.trim() });
      setName("");
      setDescription("");
    } catch {
      // Trùng tên / quá trần ký tự — server trả 400 với message đọc được.
      setFormError(true);
    }
  }

  async function toggleStatus(project: WorkProjectDto) {
    try {
      await update.mutateAsync({
        id: project.id,
        request: { status: project.status === "archived" ? "active" : "archived" },
      });
    } catch {
      // Danh sách là nguồn sự thật; lần refetch sau sẽ hiện lại trạng thái server.
    }
  }

  return (
    <Panel className="flex max-h-[calc(100svh_-_16rem)] flex-col">
      <div className="flex items-center gap-1.5">
        <h2 className="text-lg font-semibold">{t("work.projectsTitle")}</h2>
        <HelpHint title={t("work.projectsTitle")} text={t("work.projectsDescription")} />
      </div>
      <form className="mt-4 space-y-3" onSubmit={submit}>
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          className="field-input"
          placeholder={t("work.projectName")}
          aria-label={t("work.projectName")}
          maxLength={120}
          required
        />
        <input
          value={description}
          onChange={(event) => setDescription(event.target.value)}
          className="field-input"
          placeholder={t("work.projectDescription")}
          aria-label={t("work.projectDescription")}
          maxLength={4000}
        />
        {formError ? (
          <p className="text-sm text-destructive" role="alert">
            {t("work.validation")}
          </p>
        ) : null}
        <button
          type="submit"
          disabled={create.isPending}
          className="w-full rounded-lg bg-brand px-3 py-2.5 text-sm font-semibold text-brand-ink hover:bg-brand-strong disabled:opacity-50"
        >
          {create.isPending ? t("common.loading") : t("work.createProject")}
        </button>
      </form>
      <div className="mt-4 min-h-0 flex-1 overflow-y-auto pb-1">
        {state.isPending ? <LoadingState label={t("common.loading")} /> : null}
        {state.isError ? <ErrorState onRetry={() => void state.refetch()} /> : null}
        {projects.length === 0 && state.isSuccess ? (
          <EmptyState title={t("work.projectsEmpty")} description={t("work.projectsEmptyDescription")} />
        ) : null}
        <ul className="space-y-3">
          {projects.map((project) => (
            <li key={project.id} className="rounded-lg border border-border p-3">
              <div className="flex items-start justify-between gap-2">
                <p className="min-w-0 break-words font-semibold">{project.name}</p>
                <span
                  className={`shrink-0 rounded-full px-2 py-0.5 text-xs font-semibold ${
                    project.status === "archived" ? "bg-accent text-muted-foreground" : "bg-live/10 text-live"
                  }`}
                >
                  {project.status === "archived" ? t("work.projectArchived") : t("work.projectActive")}
                </span>
              </div>
              {project.description ? (
                <p className="mt-1 line-clamp-2 break-words text-sm text-muted-foreground">
                  {project.description}
                </p>
              ) : null}
              <button
                type="button"
                disabled={update.isPending}
                onClick={() => void toggleStatus(project)}
                className="mt-2 rounded-lg border border-border px-2.5 py-1 text-xs font-semibold hover:bg-accent disabled:opacity-50"
              >
                {project.status === "archived" ? t("work.unarchive") : t("work.archive")}
              </button>
            </li>
          ))}
        </ul>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Tasks — cột phải: bộ lọc, thêm việc (agent tự tạo, không cần duyệt), danh sách
// ---------------------------------------------------------------------------

function TasksPanel({ projects }: { projects: WorkProjectDto[] }) {
  const { t } = useI18n();
  const [projectFilter, setProjectFilter] = useState<number | null>(null);
  const [statusFilter, setStatusFilter] = useState<string>("");
  const tasks = useWorkTasks({
    projectId: projectFilter ?? undefined,
    status: statusFilter || undefined,
  });

  const projectName = new Map(projects.map((project) => [project.id, project.name]));

  return (
    <Panel className="flex max-h-[calc(100svh_-_16rem)] flex-col">
      <div className="flex items-center gap-1.5">
        <h2 className="text-lg font-semibold">{t("work.tasksTitle")}</h2>
        <HelpHint title={t("work.tasksTitle")} text={t("work.tasksDescription")} />
      </div>
      <div className="mt-4 grid gap-2 sm:grid-cols-2">
        <select
          value={projectFilter ?? ""}
          onChange={(event) => setProjectFilter(event.target.value ? Number(event.target.value) : null)}
          className="field-input"
          aria-label={t("work.filterProject")}
        >
          <option value="">{t("work.filterAll")}</option>
          {projects.map((project) => (
            <option key={project.id} value={project.id}>
              {project.name}
            </option>
          ))}
        </select>
        <select
          value={statusFilter}
          onChange={(event) => setStatusFilter(event.target.value)}
          className="field-input"
          aria-label={t("work.filterStatus")}
        >
          <option value="">{t("work.filterAll")}</option>
          {TASK_STATUSES.map((status) => (
            <option key={status} value={status}>
              {t(`work.status.${status}`)}
            </option>
          ))}
        </select>
      </div>
      {projects.length > 0 ? <AddTaskForm projects={projects} /> : null}
      <div className="mt-4 min-h-0 flex-1 overflow-y-auto pb-1">
        {tasks.isPending ? <LoadingState label={t("common.loading")} /> : null}
        {tasks.isError ? <ErrorState onRetry={() => void tasks.refetch()} /> : null}
        {tasks.data?.tasks.length === 0 ? (
          <EmptyState title={t("work.tasksEmpty")} description={t("work.tasksEmptyDescription")} />
        ) : null}
        <ul className="space-y-3">
          {tasks.data?.tasks.map((task) => (
            <TaskCard key={task.id} task={task} projectName={projectName.get(task.project_id) ?? "#"} />
          ))}
        </ul>
      </div>
    </Panel>
  );
}

function AddTaskForm({ projects }: { projects: WorkProjectDto[] }) {
  const { t } = useI18n();
  const create = useCreateWorkTask();
  const [projectId, setProjectId] = useState<number>(projects[0]?.id ?? 0);
  const [title, setTitle] = useState("");
  const [notes, setNotes] = useState("");
  const [assignee, setAssignee] = useState("");
  const [priority, setPriority] = useState<string>("normal");
  const [formError, setFormError] = useState(false);

  // Project đầu tiên trong danh sách là mặc định hợp lý; khi danh sách tải muộn
  // thì cập nhật lựa chọn sang project có thật (không ghi đè lựa chọn tay).
  const effectiveProjectId = projects.some((project) => project.id === projectId)
    ? projectId
    : (projects[0]?.id ?? 0);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setFormError(false);
    if (!title.trim() || !effectiveProjectId) {
      setFormError(true);
      return;
    }
    try {
      await create.mutateAsync({
        project_id: effectiveProjectId,
        title: title.trim(),
        notes: notes.trim(),
        assignee,
        priority,
      });
      setTitle("");
      setNotes("");
    } catch {
      setFormError(true);
    }
  }

  return (
    <form className="mt-4 space-y-2" onSubmit={submit}>
      <input
        value={title}
        onChange={(event) => setTitle(event.target.value)}
        className="field-input"
        placeholder={t("work.taskTitle")}
        aria-label={t("work.taskTitle")}
        maxLength={200}
        required
      />
      <div className="grid gap-2 sm:grid-cols-3">
        <select
          value={effectiveProjectId}
          onChange={(event) => setProjectId(Number(event.target.value))}
          className="field-input"
          aria-label={t("work.taskProject")}
        >
          {projects.map((project) => (
            <option key={project.id} value={project.id}>
              {project.name}
            </option>
          ))}
        </select>
        <select
          value={assignee}
          onChange={(event) => setAssignee(event.target.value)}
          className="field-input"
          aria-label={t("work.assignee")}
        >
          <option value="">{t("work.assigneeNone")}</option>
          {ASSIGNEE_ROLES.map((role) => (
            <option key={role} value={role}>
              {role}
            </option>
          ))}
        </select>
        <select
          value={priority}
          onChange={(event) => setPriority(event.target.value)}
          className="field-input"
          aria-label={t("work.priority")}
        >
          {PRIORITIES.map((level) => (
            <option key={level} value={level}>
              {t(`work.priority.${level}`)}
            </option>
          ))}
        </select>
      </div>
      <input
        value={notes}
        onChange={(event) => setNotes(event.target.value)}
        className="field-input"
        placeholder={t("work.taskNotes")}
        aria-label={t("work.taskNotes")}
        maxLength={4000}
      />
      {formError ? (
        <p className="text-sm text-destructive" role="alert">
          {t("work.validation")}
        </p>
      ) : null}
      <button
        type="submit"
        disabled={create.isPending}
        className="w-full rounded-lg bg-brand px-3 py-2 text-sm font-semibold text-brand-ink hover:bg-brand-strong disabled:opacity-50"
      >
        {create.isPending ? t("common.loading") : t("work.addTask")}
      </button>
    </form>
  );
}

const STATUS_BADGE: Record<string, string> = {
  open: "bg-accent text-muted-foreground",
  in_progress: "bg-live/10 text-live",
  blocked: "bg-need/10 text-need",
  done: "bg-live/10 text-live",
};

function TaskCard({ task, projectName }: { task: WorkTaskDto; projectName: string }) {
  const { t } = useI18n();
  const update = useUpdateWorkTask();
  const remove = useDeleteWorkTask();
  const [notesOpen, setNotesOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const statusText = statusKey(task.status) ?? task.status;
  const priorityText = priorityKey(task.priority) ?? task.priority;

  return (
    <li className="rounded-lg border border-border p-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <p className="min-w-0 break-words font-medium">{task.title}</p>
        <div className="flex shrink-0 items-center gap-2">
          <span
            className={`rounded-full px-2 py-0.5 text-xs font-semibold ${STATUS_BADGE[task.status] ?? "bg-accent text-muted-foreground"}`}
          >
            {statusText}
          </span>
          <span
            className={`rounded-full px-2 py-0.5 text-xs font-semibold ${
              task.priority === "high" ? "bg-need/10 text-need" : "bg-accent text-muted-foreground"
            }`}
          >
            {priorityText}
          </span>
        </div>
      </div>
      {task.notes ? (
        <p className="mt-1 whitespace-pre-wrap break-words text-sm text-muted-foreground">{task.notes}</p>
      ) : null}
      <dl className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <div>
          <dt className="inline font-medium">{t("work.taskProject")}: </dt>
          <dd className="inline">{projectName}</dd>
        </div>
        {task.assignee ? (
          <div>
            <dt className="inline font-medium">{t("work.assignee")}: </dt>
            <dd className="inline">{task.assignee}</dd>
          </div>
        ) : null}
        <div>
          <dt className="inline font-medium">{t("work.createdBy")}: </dt>
          <dd className="inline font-mono">{task.created_by}</dd>
        </div>
        {task.done_at ? (
          <div>
            <dt className="inline font-medium">{t("work.doneAt")}: </dt>
            <dd className="inline">{formatDateTime(task.done_at)}</dd>
          </div>
        ) : null}
      </dl>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <select
          value={task.status}
          onChange={(event) =>
            void update.mutateAsync({ id: task.id, patch: { status: event.target.value } }).catch(() => {
              // Danh sách là nguồn sự thật; giữ nguyên lựa chọn khi lỗi.
            })
          }
          disabled={update.isPending}
          className="rounded-lg border border-border bg-transparent px-2 py-1 text-xs font-semibold"
          aria-label={t("work.statusLabel")}
        >
          {TASK_STATUSES.map((status) => (
            <option key={status} value={status}>
              {t(`work.status.${status}`)}
            </option>
          ))}
        </select>
        <button
          type="button"
          onClick={() => setNotesOpen((open) => !open)}
          aria-expanded={notesOpen}
          className="rounded-lg border border-border px-2.5 py-1 text-xs font-semibold hover:bg-accent"
        >
          {t("work.notes")}
        </button>
        <button
          type="button"
          onClick={() => setConfirmDelete(true)}
          className="rounded-lg border border-destructive/40 px-2.5 py-1 text-xs font-semibold text-destructive hover:bg-destructive/20"
        >
          {t("common.delete")}
        </button>
      </div>
      {notesOpen ? <TaskNotes taskId={task.id} /> : null}
      <ConfirmDialog
        open={confirmDelete}
        title={t("work.deleteTaskTitle")}
        message={t("work.deleteTaskMessage")}
        confirmLabel={t("common.delete")}
        cancelLabel={t("common.cancel")}
        danger
        pending={remove.isPending}
        onConfirm={() => {
          void remove
            .mutateAsync(task.id)
            .then(() => setConfirmDelete(false))
            .catch(() => {
              // Giữ hộp xác nhận mở khi xoá lỗi để người dùng thử lại.
            });
        }}
        onCancel={() => setConfirmDelete(false)}
      />
    </li>
  );
}

function TaskNotes({ taskId }: { taskId: number }) {
  const { t } = useI18n();
  const notes = useWorkNotes(taskId, true);
  const add = useAddWorkNote();
  const [text, setText] = useState("");

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!text.trim()) return;
    try {
      await add.mutateAsync({ taskId, request: { text: text.trim() } });
      setText("");
    } catch {
      // Giữ nguyên nội dung đã gõ để người dùng thử lại.
    }
  }

  return (
    <div className="mt-3 rounded-lg bg-accent/50 p-3">
      <form className="flex gap-2" onSubmit={submit}>
        <input
          value={text}
          onChange={(event) => setText(event.target.value)}
          className="field-input flex-1"
          placeholder={t("work.notePlaceholder")}
          aria-label={t("work.addNote")}
          maxLength={4000}
        />
        <button
          type="submit"
          disabled={add.isPending || !text.trim()}
          className="rounded-lg bg-brand px-3 py-1.5 text-xs font-semibold text-brand-ink hover:bg-brand-strong disabled:opacity-50"
        >
          {t("work.addNote")}
        </button>
      </form>
      {notes.isPending ? <LoadingState label={t("common.loading")} /> : null}
      {notes.isError ? <ErrorState onRetry={() => void notes.refetch()} /> : null}
      {notes.data?.notes.length === 0 ? (
        <p className="mt-2 text-xs text-muted-foreground">{t("work.noNotes")}</p>
      ) : null}
      <ul className="mt-2 space-y-2">
        {notes.data?.notes.map((note) => (
          <li key={note.id} className="rounded-md border border-border p-2">
            <p className="whitespace-pre-wrap break-words text-sm">{note.text}</p>
            <p className="mt-1 text-xs text-muted-foreground">
              <span className="font-mono">{note.author}</span> · {formatDateTime(note.created_at)}
            </p>
          </li>
        ))}
      </ul>
    </div>
  );
}
