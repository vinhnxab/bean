import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useParams } from "react-router";

import type { DecisionDto, MessageDto } from "@/api/bindings";
import { BeanAvatar } from "@/components/brand/BeanAvatar";
import { ConfirmCard } from "@/components/chat/ConfirmCard";
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
    element.scrollTop = element.scrollHeight;
  }, [scrollKey]);

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
      <header className="border-b border-slate-200 bg-white/80 px-4 py-3 backdrop-blur md:px-6 dark:border-slate-800 dark:bg-slate-900/80">
        <div className="mx-auto flex max-w-4xl items-center justify-between gap-3">
          <h1 className="truncate text-lg font-semibold">{t("chat.sessions")}</h1>
          <ConnectionBadge status={status} />
        </div>
      </header>
      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto px-4 py-5 md:px-6" aria-live="polite">
        <div className="mx-auto max-w-4xl">
          {history.isPending ? (
            <p className="py-8 text-center text-sm text-slate-500">{t("common.loading")}</p>
          ) : null}
          {history.isError ? <HistoryError onRetry={() => void history.refetch()} /> : null}
          {history.hasNextPage ? (
            <button
              type="button"
              onClick={loadOlder}
              disabled={history.isFetchingNextPage}
              className="mx-auto mb-5 block rounded-full border border-slate-300 px-4 py-2 text-sm hover:bg-white disabled:opacity-50 dark:border-slate-700 dark:hover:bg-slate-900"
            >
              {history.isFetchingNextPage ? t("common.loading") : t("chat.loadingOlder")}
            </button>
          ) : messages.length > 0 ? (
            <p className="mb-5 text-center text-xs text-slate-400">{t("chat.noMore")}</p>
          ) : null}
          {!history.isPending && messages.length === 0 ? (
            <div className="py-16 text-center text-sm text-slate-500">{t("chat.emptyDescription")}</div>
          ) : null}
          <div className="space-y-5">
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
              <div className="rounded-2xl bg-slate-100 px-4 py-3 dark:bg-slate-800">
                <SafeMarkdown>{run.streamText}</SafeMarkdown>
              </div>
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
            <p className="mt-4 text-center text-sm text-amber-700 dark:text-amber-300" role="status">
              {t("chat.queued")} · #{run.queuePosition}
            </p>
          ) : null}
          {run?.status === "running" ? (
            <p className="mt-4 text-center text-sm text-emerald-700 dark:text-emerald-300" role="status">
              {t("chat.running")}
            </p>
          ) : null}
          {run?.error ? (
            <p
              className="mt-4 rounded-xl bg-rose-50 p-3 text-sm text-rose-700 dark:bg-rose-950 dark:text-rose-300"
              role="alert"
            >
              {run.error}
            </p>
          ) : null}
        </div>
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

function Composer({
  value,
  onChange,
  onSubmit,
  onStop,
  canStop,
  stopping,
  disabled,
  sendError,
}: {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  onStop: () => void;
  canStop: boolean;
  stopping: boolean;
  disabled: boolean;
  sendError: boolean;
}) {
  const { t } = useI18n();
  return (
    <div className="border-t border-slate-200 bg-white px-4 py-3 md:px-6 dark:border-slate-800 dark:bg-slate-900">
      <div className="mx-auto max-w-4xl">
        <div className="flex items-end gap-2">
          <textarea
            value={value}
            onChange={(event) => onChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                onSubmit();
              }
            }}
            rows={2}
            placeholder={t("chat.placeholder")}
            disabled={disabled}
            className="min-h-12 flex-1 resize-y rounded-xl border border-slate-300 bg-white px-3 py-2 outline-none focus:border-emerald-500 focus:ring-2 focus:ring-emerald-500/20 dark:border-slate-700 dark:bg-slate-950"
            aria-label={t("chat.placeholder")}
          />
          {canStop ? (
            <button
              type="button"
              onClick={onStop}
              disabled={stopping}
              className="rounded-xl border border-rose-300 px-3 py-2 text-sm font-semibold text-rose-700 hover:bg-rose-50 disabled:opacity-50 dark:border-rose-800 dark:text-rose-300 dark:hover:bg-rose-950"
            >
              {stopping ? t("chat.stopping") : t("chat.stop")}
            </button>
          ) : (
            <button
              type="button"
              onClick={onSubmit}
              disabled={disabled || !value.trim()}
              className="rounded-xl bg-emerald-600 px-4 py-2 text-sm font-semibold text-white hover:bg-emerald-700 disabled:opacity-50"
            >
              {t("chat.send")}
            </button>
          )}
        </div>
        {sendError ? (
          <p className="mt-2 text-sm text-rose-600" role="alert">
            {t("chat.sendError")}
          </p>
        ) : null}
        <p className="mt-1 text-right text-xs text-slate-400">{t("chat.enterHint")}</p>
      </div>
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
      className={`inline-flex items-center gap-1 text-xs ${status === "connected" ? "text-emerald-600 dark:text-emerald-400" : "text-amber-600 dark:text-amber-400"}`}
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
    <div className="flex flex-1 items-center justify-center p-6 text-sm text-rose-600">
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
  const { t } = useI18n();
  // Avatar chỉ ở bên Manager: tin của bạn đã căn phải, thêm ảnh sẽ thành hai
  // hàng lệch nhau và làm rối mốc đọc "ai đang nói".
  return (
    <div className={`flex gap-2.5 ${messageRole === "user" ? "justify-end" : "justify-start"}`}>
      {messageRole === "user" ? null : (
        <BeanAvatar size={28} className="mt-0.5" title={t("hub.manager.name")} />
      )}
      <div
        className={`max-w-[85%] rounded-2xl px-4 py-3 ${messageRole === "user" ? "bg-emerald-600 text-white" : "bg-white text-slate-900 shadow-sm dark:bg-slate-900 dark:text-slate-100"}`}
      >
        <SafeMarkdown>{children}</SafeMarkdown>
      </div>
    </div>
  );
}

function HistoryError({ onRetry }: { onRetry: () => void }) {
  const { t } = useI18n();
  return (
    <div
      className="rounded-xl bg-rose-50 p-4 text-sm text-rose-700 dark:bg-rose-950 dark:text-rose-300"
      role="alert"
    >
      {t("chat.historyError")}{" "}
      <button type="button" className="ml-2 font-semibold underline" onClick={onRetry}>
        {t("common.retry")}
      </button>
    </div>
  );
}
