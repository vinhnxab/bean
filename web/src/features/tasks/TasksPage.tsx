import { type FormEvent, useMemo, useState } from "react";

import type { TaskDto } from "@/api/bindings";
import { HelpHint } from "@/components/HelpHint";
import { ConfirmDialog } from "@/components/ui/ConfirmDialog";
import { EmptyState, ErrorState, LoadingState, PageShell, Panel } from "@/components/ui/Page";
import { useCreateTask, useDeleteTask, useTasks, useUpdateTask } from "@/features/tasks/queries";
import { useTools } from "@/features/tools/queries";
import { useI18n } from "@/i18n";
import { formatDateTime } from "@/lib/format";

type TaskForm = {
  cron: string;
  prompt: string;
  channel: string;
  chatId: string;
  allowedTools: string;
  enabled: boolean;
};

const INITIAL_FORM: TaskForm = {
  cron: "0 9 * * *",
  prompt: "",
  channel: "web",
  chatId: "web:admin",
  allowedTools: "",
  enabled: true,
};

export function TasksPage() {
  const { t } = useI18n();
  const tasks = useTasks();
  const create = useCreateTask();
  const update = useUpdateTask();
  const remove = useDeleteTask();
  const [form, setForm] = useState<TaskForm>(INITIAL_FORM);
  const [target, setTarget] = useState<TaskDto | null>(null);
  const [formError, setFormError] = useState(false);
  const tools = useTools();

  // S4: `allowed_tools` cấp quyền vĩnh viễn cho run định kỳ không có người duyệt —
  // cảnh báo ngay khi người dùng nhập tên tool có mức rủi ro Dangerous.
  const dangerousTools = useMemo(() => {
    const registry = tools.data;
    if (!registry) return [];
    const riskOf = new Map(registry.tools.map((tool) => [tool.name, tool.risk]));
    return form.allowedTools
      .split(",")
      .map((tool) => tool.trim())
      .filter((tool) => riskOf.get(tool) === "dangerous");
  }, [tools.data, form.allowedTools]);

  const dangerousToolNames = useMemo(() => {
    const registry = tools.data;
    if (!registry) return new Set<string>();
    return new Set(registry.tools.filter((tool) => tool.risk === "dangerous").map((tool) => tool.name));
  }, [tools.data]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setFormError(false);
    if (!form.cron.trim() || !form.prompt.trim() || !form.channel.trim() || !form.chatId.trim()) {
      setFormError(true);
      return;
    }
    try {
      await create.mutateAsync({
        session_id: null,
        cron: form.cron.trim(),
        prompt: form.prompt.trim(),
        channel: form.channel.trim(),
        chat_id: form.chatId.trim(),
        allowed_tools: form.allowedTools
          .split(",")
          .map((tool) => tool.trim())
          .filter(Boolean),
        enabled: form.enabled,
      });
      setForm((current) => ({ ...current, prompt: "" }));
    } catch {
      setFormError(true);
    }
  }

  async function toggle(task: TaskDto) {
    try {
      await update.mutateAsync({ id: task.id, request: { enabled: !task.enabled } });
    } catch {
      // The list remains the source of truth; the next refetch will show the server state.
    }
  }

  async function confirmDelete() {
    if (!target) return;
    try {
      await remove.mutateAsync(target.id);
      setTarget(null);
    } catch {
      // Keep the confirmation open when the request fails.
    }
  }

  return (
    <PageShell title={t("tasks.title")} description={t("tasks.description")}>
      <div className="grid gap-6 xl:grid-cols-[minmax(18rem,0.8fr)_minmax(0,1.2fr)]">
        <Panel>
          <div className="flex items-center gap-1.5">
            <h2 className="text-lg font-semibold">{t("tasks.createTitle")}</h2>
            <HelpHint title={t("tasks.createTitle")} text={t("tasks.createDescription")} />
          </div>
          <form className="mt-5 space-y-4" onSubmit={submit}>
            <Field label={t("tasks.cron")} id="task-cron" help={t("tasks.cronHelp")}>
              <input
                id="task-cron"
                value={form.cron}
                onChange={(event) => setForm({ ...form, cron: event.target.value })}
                className="field-input"
                required
              />
            </Field>
            <Field label={t("tasks.prompt")} id="task-prompt" help={t("tasks.promptHelp")}>
              <textarea
                id="task-prompt"
                value={form.prompt}
                onChange={(event) => setForm({ ...form, prompt: event.target.value })}
                rows={5}
                className="field-input resize-y"
                required
              />
            </Field>
            <div className="grid gap-4 sm:grid-cols-2">
              <Field label={t("tasks.channel")} id="task-channel">
                <input
                  id="task-channel"
                  value={form.channel}
                  onChange={(event) => setForm({ ...form, channel: event.target.value })}
                  className="field-input"
                  required
                />
              </Field>
              <Field label={t("tasks.chatId")} id="task-chat-id">
                <input
                  id="task-chat-id"
                  value={form.chatId}
                  onChange={(event) => setForm({ ...form, chatId: event.target.value })}
                  className="field-input"
                  required
                />
              </Field>
            </div>
            <Field label={t("tasks.allowedTools")} id="task-tools" help={t("tasks.allowedToolsHelp")}>
              <input
                id="task-tools"
                value={form.allowedTools}
                onChange={(event) => setForm({ ...form, allowedTools: event.target.value })}
                className="field-input"
                placeholder="run_shell, web_search"
              />
            </Field>
            {dangerousTools.length > 0 ? (
              <p className="text-sm text-need" role="alert">
                {t("tasks.dangerousToolsWarning")}: {dangerousTools.join(", ")}
              </p>
            ) : null}
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={form.enabled}
                onChange={(event) => setForm({ ...form, enabled: event.target.checked })}
              />
              {t("tasks.enabled")}
            </label>
            {formError ? (
              <p className="text-sm text-destructive" role="alert">
                {t("tasks.validation")}
              </p>
            ) : null}
            <button
              type="submit"
              disabled={create.isPending}
              className="w-full rounded-lg bg-brand px-3 py-2.5 text-sm font-semibold text-brand-ink hover:bg-brand-strong disabled:opacity-50"
            >
              {create.isPending ? t("common.loading") : t("tasks.create")}
            </button>
          </form>
        </Panel>
        <Panel>
          <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-1.5">
              <h2 className="text-lg font-semibold">{t("tasks.listTitle")}</h2>
              <HelpHint title={t("tasks.listTitle")} text={t("tasks.runtimeNote")} />
            </div>
          </div>
          {tasks.isPending ? <LoadingState label={t("common.loading")} /> : null}
          {tasks.isError ? <ErrorState onRetry={() => void tasks.refetch()} /> : null}
          {tasks.data?.tasks.length === 0 ? (
            <EmptyState title={t("tasks.empty")} description={t("tasks.emptyDescription")} />
          ) : null}
          <ul className="space-y-3">
            {tasks.data?.tasks.map((task) => (
              <li key={task.id} className="rounded-lg border border-border p-4">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="font-mono text-sm font-semibold">{task.cron}</p>
                    <p className="mt-1 whitespace-pre-wrap break-words text-sm">{task.prompt}</p>
                  </div>
                  <span
                    className={`rounded-full px-2 py-1 text-xs font-semibold ${task.enabled ? "bg-live/10 text-live" : "bg-accent text-muted-foreground"}`}
                  >
                    {task.enabled ? t("tasks.enabled") : t("tasks.disabled")}
                  </span>
                </div>
                <dl className="mt-3 grid gap-1 text-xs text-muted-foreground sm:grid-cols-2">
                  <div>
                    <dt className="inline font-medium">{t("tasks.nextRun")}: </dt>
                    <dd className="inline">{formatDateTime(task.next_run)}</dd>
                  </div>
                  <div>
                    <dt className="inline font-medium">{t("tasks.destination")}: </dt>
                    <dd className="inline">
                      {task.channel} / {task.chat_id}
                    </dd>
                  </div>
                </dl>
                {task.allowed_tools.length > 0 ? (
                  <p className="mt-2 text-xs text-muted-foreground">
                    {t("tasks.allowedTools")}:{" "}
                    {task.allowed_tools.map((tool, index) => (
                      <span key={tool}>
                        {index > 0 ? ", " : ""}
                        <span
                          className={dangerousToolNames.has(tool) ? "font-semibold text-need" : undefined}
                        >
                          {tool}
                        </span>
                      </span>
                    ))}
                  </p>
                ) : null}
                <div className="mt-4 flex flex-wrap gap-2">
                  <button
                    type="button"
                    disabled={update.isPending}
                    onClick={() => void toggle(task)}
                    className="rounded-lg border border-border px-3 py-1.5 text-sm font-semibold hover:bg-accent disabled:opacity-50"
                  >
                    {task.enabled ? t("tasks.disable") : t("tasks.enable")}
                  </button>
                  <button
                    type="button"
                    onClick={() => setTarget(task)}
                    className="rounded-lg border border-destructive/40 px-3 py-1.5 text-sm font-semibold text-destructive hover:bg-destructive/20"
                  >
                    {t("common.delete")}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </Panel>
      </div>
      <ConfirmDialog
        open={target !== null}
        title={t("tasks.deleteTitle")}
        message={target ? `${t("tasks.deleteMessage")}\n${target.prompt}` : ""}
        confirmLabel={t("common.delete")}
        cancelLabel={t("common.cancel")}
        danger
        pending={remove.isPending}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setTarget(null)}
      />
    </PageShell>
  );
}

function Field({
  label,
  id,
  help,
  children,
}: {
  label: string;
  id: string;
  help?: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      {/* Help text thành dấu hỏi cạnh label thay vì dòng chữ dưới ô nhập — form
          gọn một dòng mỗi field. Hint là SIBLING của label (không lồng vào):
          bấm vào label phải focus input, bấm vào dấu hỏi phải mở popover. */}
      <div className="mb-1 flex items-center gap-1.5">
        <label htmlFor={id} className="text-sm font-medium">
          {label}
        </label>
        {help ? <HelpHint title={label} text={help} /> : null}
      </div>
      {children}
    </div>
  );
}
