import { ChevronDownIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useParams } from "react-router";
import type { DecisionDto, MessageDto } from "@/api/bindings";
import { BeanMark } from "@/components/brand/BeanMark";
import { Composer } from "@/components/chat/Composer";
import { ConfirmCard } from "@/components/chat/ConfirmCard";
import { MessageRow } from "@/components/chat/MessageRow";
import { ToolCard } from "@/components/chat/ToolCard";
import { SafeMarkdown } from "@/components/markdown/SafeMarkdown";
import {
  deduplicateMessages,
  formatJson,
  parseStoredMessage,
  type StoredMessage,
  type StoredToolCall,
  textFromDto,
} from "@/features/chat/messages";
import { useSessionMessages } from "@/features/chat/queries";
import { useRealtime } from "@/features/chat/RealtimeProvider";
import { useI18n } from "@/i18n";
import { useChatStore } from "@/store/chat";

export function ChatPage() {
  const { sessionId: rawSessionId } = useParams();
  const sessionId = Number(rawSessionId);
  if (!Number.isSafeInteger(sessionId) || sessionId <= 0) return <InvalidSession />;
  return <ChatSession sessionId={sessionId} />;
}

function ChatSession({ sessionId }: { sessionId: number }) {
  const { t } = useI18n();
  const history = useSessionMessages(sessionId);
  const { status, sendStart, sendCancel, sendConfirm } = useRealtime();
  const run = useChatStore((state) => state.runsBySession[sessionId]);
  const allConfirms = useChatStore((state) => state.confirmsById);
  const expireConfirm = useChatStore((state) => state.expireConfirm);
  const [text, setText] = useState("");
  const [sendError, setSendError] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const previousHeight = useRef<number | null>(null);
  // `atBottom` theo dõi vị trí cuộn để (1) không tự cuộn khi người dùng đang đọc
  // lại nội dung cũ, (2) hiện nút "xuống đáy" khi lệch khỏi đáy.
  const [atBottom, setAtBottom] = useState(true);
  const messages = useMemo(
    () => deduplicateMessages(history.data?.pages.flatMap((page) => page.messages) ?? []),
    [history.data?.pages],
  );
  const toolResults = useMemo(() => buildToolResults(messages), [messages]);
  const liveToolIds = useMemo(() => new Set(Object.keys(run?.liveTools ?? {})), [run?.liveTools]);
  const confirms = useMemo(
    () => Object.values(allConfirms).filter((item) => item.session_id === sessionId),
    [allConfirms, sessionId],
  );

  const scrollKey = `${messages.length}:${run?.streamText ?? ""}:${Object.keys(run?.liveTools ?? {}).length}`;

  useEffect(() => {
    void scrollKey;
    const element = scrollRef.current;
    if (!element) return;
    if (previousHeight.current !== null) {
      element.scrollTop = element.scrollHeight - previousHeight.current;
      previousHeight.current = null;
      return;
    }
    // Chỉ cuộn theo khi người dùng **đang ở đáy**. Họ cuộn lên để đọc lại một
    // đoạn nào đó thì tin nhắm mới (scheduler, tin chủ động) không được giật
    // màn hình họ ra khỏi chỗ đang đọc.
    if (atBottom) element.scrollTop = element.scrollHeight;
  }, [atBottom, scrollKey]);

  /** Người dùng tự cuộn: biết mình đang ở đâu để quyết định có cuộn theo không. */
  function onScroll() {
    const element = scrollRef.current;
    if (!element) return;
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    setAtBottom(distance < 48);
  }

  function scrollToBottom() {
    const element = scrollRef.current;
    if (element) element.scrollTop = element.scrollHeight;
    setAtBottom(true);
  }

  const decide = useCallback(
    (confirmId: string, decision: DecisionDto) => {
      if (!sendConfirm(confirmId, decision)) setSendError(true);
    },
    [sendConfirm],
  );
  const expire = useCallback((confirmId: string) => expireConfirm(confirmId), [expireConfirm]);

  function submit() {
    const value = text.trim();
    if (!value || status !== "connected") {
      if (value) setSendError(true);
      return;
    }
    if (!sendStart(sessionId, value)) {
      setSendError(true);
      return;
    }
    setText("");
    setSendError(false);
  }

  function loadOlder() {
    const element = scrollRef.current;
    if (element) previousHeight.current = element.scrollHeight;
    void history.fetchNextPage();
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Thanh tiêu đề: `sticky` + gradient mờ dần thay cho `border-b` cứng, để
          nội dung chạy dưới mà không bị cắt ngang bởi một đường kẻ. */}
      <header className="sticky top-0 z-10 bg-gradient-to-b from-surface via-surface to-transparent px-4 pb-4 pt-3 md:px-6">
        <div className="mx-auto flex max-w-4xl items-center justify-between gap-3">
          <h1 className="truncate text-sm font-semibold text-ink-muted">{t("chat.sessions")}</h1>
          <ConnectionBadge status={status} />
        </div>
      </header>
      <div className="relative min-h-0 flex-1">
        <div
          ref={scrollRef}
          onScroll={onScroll}
          className="h-full overflow-y-auto px-2 pb-4 md:px-4"
          aria-live="polite"
          aria-relevant="additions text"
        >
          <div className="mx-auto max-w-4xl">
            {history.isPending ? (
              <p className="py-8 text-center text-sm text-muted-foreground">{t("common.loading")}</p>
            ) : null}
            {history.isError ? <HistoryError onRetry={() => void history.refetch()} /> : null}
            {history.hasNextPage ? (
              <button
                type="button"
                onClick={loadOlder}
                disabled={history.isFetchingNextPage}
                className="mx-auto mb-5 block rounded-full border border-rule px-4 py-2 text-sm text-muted-foreground transition-colors hover:bg-surface-raised hover:text-ink disabled:opacity-50"
              >
                {history.isFetchingNextPage ? t("common.loading") : t("chat.loadingOlder")}
              </button>
            ) : null}
            {!history.isPending && messages.length === 0 ? <EmptyChat onPick={setText} /> : null}
            <div className="space-y-1">
              {messages.map((message) => (
                <HistoryMessage
                  key={message.id}
                  dto={message}
                  toolResults={toolResults}
                  liveToolIds={liveToolIds}
                />
              ))}
              {Object.values(run?.liveTools ?? {})
                .filter((tool) => !toolResults.has(tool.id))
                .map((tool) => (
                  <ToolCard
                    key={`${tool.runId}:${tool.id}`}
                    name={tool.tool}
                    summary={tool.summary}
                    argsPreview={tool.argsPreview}
                    outputPreview={tool.outputPreview}
                    status={tool.status}
                  />
                ))}
              {run?.streamText ? (
                <MessageRow author="assistant" headingId="stream-text">
                  <SafeMarkdown>{run.streamText}</SafeMarkdown>
                </MessageRow>
              ) : null}
              {confirms.map((confirm) => (
                <ConfirmCard
                  key={confirm.confirm_id}
                  confirm={confirm}
                  onDecision={(decision) => decide(confirm.confirm_id, decision)}
                  onExpire={expire}
                />
              ))}
            </div>
            {run?.status === "queued" ? (
              <p className="mt-4 text-center text-sm text-need" role="status">
                {t("chat.queued")} · #{run.queuePosition}
              </p>
            ) : null}
            {run?.status === "running" ? (
              <p className="mt-4 text-center text-sm text-live" role="status">
                {t("chat.running")}
              </p>
            ) : null}
            {run?.error ? (
              <p className="mt-4 rounded-lg bg-alert/10 p-3 text-sm text-alert" role="alert">
                {run.error}
              </p>
            ) : null}
          </div>
        </div>
        {/* Nút tròn nổi lên khi lệch khỏi đáy — theo mẫu Open WebUI. Chỉ hiện
            khi thực sự cần, không chiếm chỗ khi đang ở đáy. */}
        {atBottom ? null : (
          <button
            type="button"
            onClick={scrollToBottom}
            aria-label={t("chat.scrollToBottom")}
            title={t("chat.scrollToBottom")}
            className="absolute bottom-4 left-1/2 z-10 flex size-9 -translate-x-1/2 items-center justify-center rounded-full border border-rule bg-surface-raised text-ink shadow-md transition-colors hover:bg-surface-hover"
          >
            <ChevronDownIcon className="size-4" />
          </button>
        )}
      </div>
      <Composer
        value={text}
        onChange={setText}
        onSubmit={submit}
        onStop={() => sendCancel(sessionId)}
        canStop={run !== undefined && run.status !== "idle" && run.status !== "submitting"}
        stopping={run?.status === "stopping"}
        disabled={status !== "connected"}
        sendError={sendError}
      />
    </div>
  );
}

/**
 * Màn hình rỗng: mascot + lời chào + lưới gợi ý.
 *
 * Bốn thẻ gợi ý làm hai việc: chỉ cho biết Bean làm được gì, và cho người dùng
 * một cách bắt đầu mà không phải nghĩ câu lệnh. Nhấn thẻ **điền sẵn vào ô
 * soạn** chứ không gửi thẳng — người dùng luôn có quyền sửa trước khi gửi,
 * và hành vi đó nhất quán với việc ô soạn luôn rỗng sau khi gửi.
 */
function EmptyChat({ onPick }: { onPick: (prompt: string) => void }) {
  const { t } = useI18n();
  const suggestions = [
    t("chat.suggestion.plan"),
    t("chat.suggestion.explore"),
    t("chat.suggestion.memory"),
    t("chat.suggestion.summarize"),
  ];
  return (
    <div className="flex flex-col items-center px-2 py-16 text-center">
      <BeanMark size={56} className="mb-4 text-brand" title={t("app.title")} />
      <h2 className="text-2xl font-semibold tracking-tight">
        {t("chat.greeting")} <span className="text-brand">Bean</span>
      </h2>
      <p className="mt-2 text-sm text-muted-foreground">{t("chat.emptyHint")}</p>
      <ul className="mt-8 grid w-full max-w-2xl gap-2 sm:grid-cols-2" aria-label={t("chat.suggestionsLabel")}>
        {suggestions.map((prompt) => (
          <li key={prompt}>
            <button
              type="button"
              onClick={() => onPick(prompt)}
              className="w-full rounded-lg border border-rule bg-surface-raised px-4 py-3 text-left text-sm text-ink transition-colors hover:border-brand hover:bg-surface-hover"
            >
              {prompt}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function ConnectionBadge({ status }: { status: "idle" | "connecting" | "connected" | "reconnecting" }) {
  const { t } = useI18n();
  const label =
    status === "connected"
      ? t("chat.connected")
      : status === "reconnecting" || status === "connecting"
        ? t("chat.reconnecting")
        : t("chat.offline");
  return (
    <span
      className={`inline-flex items-center gap-1 text-xs ${status === "connected" ? "text-live" : "text-need"}`}
      role="status"
    >
      <span className="h-2 w-2 rounded-full bg-current" aria-hidden="true" />
      {label}
    </span>
  );
}

function InvalidSession() {
  const { t } = useI18n();
  return (
    <div className="flex flex-1 items-center justify-center p-6 text-sm text-destructive">
      {t("chat.invalidSession")}
    </div>
  );
}

function buildToolResults(messages: MessageDto[]): Map<string, { call: StoredToolCall; result: MessageDto }> {
  const calls = new Map<string, StoredToolCall>();
  for (const dto of messages) {
    const parsed = parseStoredMessage(dto);
    if (parsed?.role === "assistant") for (const call of parsed.toolCalls) calls.set(call.id, call);
  }
  const result = new Map<string, { call: StoredToolCall; result: MessageDto }>();
  for (const dto of messages) {
    const parsed = parseStoredMessage(dto);
    const call = parsed?.role === "tool" ? calls.get(parsed.toolCallId ?? "") : undefined;
    if (call) result.set(call.id, { call, result: dto });
  }
  return result;
}

function HistoryMessage({
  dto,
  toolResults,
  liveToolIds,
}: {
  dto: MessageDto;
  toolResults: Map<string, { call: StoredToolCall; result: MessageDto }>;
  liveToolIds: Set<string>;
}) {
  const parsed = parseStoredMessage(dto);
  if (!parsed) return null;
  if (parsed.role === "tool") {
    const result = toolResults.get(parsed.toolCallId ?? "");
    return result ? (
      <ToolCard
        name={result.call.name}
        summary={result.call.name}
        argsPreview={formatJson(result.call.args)}
        outputPreview={textFromDto(result.result)}
        status={parsed.isError ? "error" : "ok"}
        messageId={dto.id}
        image={parsed.image}
      />
    ) : null;
  }
  if (parsed.role === "assistant" && parsed.toolCalls.length > 0) {
    return (
      <div className="space-y-3">
        {parsed.text ? <MessageBubble messageRole="assistant">{parsed.text}</MessageBubble> : null}
        {parsed.toolCalls.map((call) => {
          const result = toolResults.get(call.id);
          return result || liveToolIds.has(call.id) ? null : (
            <ToolCard
              key={call.id}
              name={call.name}
              summary={call.name}
              argsPreview={formatJson(call.args)}
              outputPreview=""
              status="running"
            />
          );
        })}
      </div>
    );
  }
  return <MessageBubble messageRole={parsed.role}>{parsed.text}</MessageBubble>;
}

function MessageBubble({ messageRole, children }: { messageRole: StoredMessage["role"]; children: string }) {
  // Tin lịch sử đi qua `MessageRow` để có tên người gửi + nút sao chép khi rê.
  // `role="tool"` không render ở đây — kết quả tool nằm trong `ToolCard`.
  if (messageRole === "tool") return null;
  return (
    <MessageRow author={messageRole} headingId={`msg-${messageRole}-${children.slice(0, 8)}`}>
      <SafeMarkdown>{children}</SafeMarkdown>
    </MessageRow>
  );
}

function HistoryError({ onRetry }: { onRetry: () => void }) {
  const { t } = useI18n();
  return (
    <div className="rounded-lg bg-destructive/10 p-4 text-sm text-destructive" role="alert">
      {t("chat.historyError")}{" "}
      <button type="button" className="ml-2 font-semibold underline" onClick={onRetry}>
        {t("common.retry")}
      </button>
    </div>
  );
}
