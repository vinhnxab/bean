import { useNavigate } from "react-router";

import { useCreateSession, useSessions } from "@/features/sessions/queries";
import { useI18n } from "@/i18n";

export function ChatIndexPage() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const sessions = useSessions();
  const createSession = useCreateSession();

  async function createAndOpen() {
    const session = await createSession.mutateAsync({ title: null });
    navigate(`/sessions/${session.id}`);
  }

  return (
    <section className="flex flex-1 items-center justify-center p-6" aria-labelledby="empty-chat-title">
      <div className="max-w-md text-center">
        <div className="mx-auto mb-5 flex h-14 w-14 items-center justify-center rounded-lg brand-wash text-2xl font-bold text-brand">
          B
        </div>
        <h1 id="empty-chat-title" className="text-2xl font-semibold">
          {t("chat.emptyTitle")}
        </h1>
        <p className="mt-3 text-muted-foreground">
          {sessions.data?.sessions.length ? t("chat.emptyDescription") : t("chat.newChat")}
        </p>
        <button
          type="button"
          onClick={() => void createAndOpen()}
          disabled={createSession.isPending}
          className="mt-5 rounded-lg bg-brand px-4 py-2 text-sm font-semibold text-brand-ink disabled:opacity-50"
        >
          {createSession.isPending ? t("common.loading") : t("chat.newChat")}
        </button>
      </div>
    </section>
  );
}
