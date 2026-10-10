import gsap from "gsap";
import { ChevronDownIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useParams } from "react-router";
import type { DecisionDto, MessageDto } from "@/api/bindings";
import { BeanMark } from "@/components/brand/BeanMark";
import { Composer } from "@/components/chat/Composer";
import { ConfirmCard } from "@/components/chat/ConfirmCard";
import { MessageRow } from "@/components/chat/MessageRow";
import { QueuePill, StreamingCaret, ThinkingIndicator } from "@/components/chat/RunStatus";
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
import { Enter, type ScrollTween, tweenToBottom, useGsapAnimate } from "@/lib/anim";
import { useChatStore } from "@/store/chat";

/** Trạng thái "lịch sử nào đã sẵn có khi mở phiên" — reset theo `sessionId`. */
type SeedState = { sessionId: number; done: boolean; ids: Set<number> };

function seedTracker(sessionId: number): SeedState {
  return { sessionId, done: false, ids: new Set<number>() };
}

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

  const liveToolCount = liveToolIds.size;
  const confirmCount = confirms.length;

  // --- Hoạt ảnh "tin mới" --------------------------------------------------
  // Lịch sử sẵn có khi mở phiên KHÔNG animate: mở một phiên dài mà mỗi tin lại
  // trượt vào một lần là một cascade hỗn loạn. Chỉ tin xuất hiện SAU lần hydrate
  // đầu mới animate (`isNew`). Component này không unmount khi đổi hội thoại
  // (cùng vị trí route), nên state theo phiên phải reset ngay tại render.
  const seeded = useRef<SeedState | null>(null);
  // Khối lượng nội dung để effect cuộn biết có thứ mới được CHÈN vào.
  const volume = useRef({ messages: 0, tools: 0, confirms: 0, hydrated: false });
  const scrollTween = useRef<ScrollTween | null>(null);
  // Đặt khi người dùng bấm nút xuống đáy: effect kế tiếp phải nhả tay để tween
  // trượt nốt, không đè xuống nhảy tức thì.
  const manualScroll = useRef(false);
  if (seeded.current === null || seeded.current.sessionId !== sessionId) {
    seeded.current = seedTracker(sessionId);
    volume.current = { messages: 0, tools: 0, confirms: 0, hydrated: false };
    manualScroll.current = false;
    scrollTween.current?.kill();
    scrollTween.current = null;
  }

  // Ghi nhận lịch sử sẵn có sau lần tải đầu: những tin này là "nền cũ", không
  // hoạt ảnh. Chờ cả `isFetching` để không chốt giữa hai trang của cùng lần tải.
  useEffect(() => {
    const state = seeded.current;
    if (!state || state.done || history.isPending || history.isFetching) return;
    state.done = true;
    for (const message of messages) state.ids.add(message.id);
  }, [history.isPending, history.isFetching, messages]);

  // Đổi hội thoại: về đáy và dọn tween còn treo — phiên mới mở đúng chỗ bắt đầu,
  // không vướng vị trí cuộn của phiên trước.
  useEffect(() => {
    void sessionId; // dependency kích hoạt có chủ đích — thân effect không đọc giá trị này
    setAtBottom(true);
    scrollTween.current?.kill();
    scrollTween.current = null;
  }, [sessionId]);

  function isNew(id: number): boolean {
    const state = seeded.current;
    return state?.done === true && !state.ids.has(id);
  }

  // Trạng thái run cũng nằm trong khóa: gửi tin nhắn là chủ đích "xem câu trả
  // lời mới nhất", nên suy nghĩ/xếp hàng phải nằm trong khung nhìn ngay sau commit
  // render — không được chờ tới delta đầu tiên mới nhảy xuống.
  const scrollKey = `${messages.length}:${run?.streamText ?? ""}:${liveToolCount}:${confirmCount}:${run?.status ?? "idle"}:${run?.queuePosition ?? ""}`;

  useEffect(() => {
    void scrollKey;
    const element = scrollRef.current;
    if (!element) return;
    const before = volume.current;
    const appended =
      messages.length > before.messages || liveToolCount > before.tools || confirmCount > before.confirms;
    // Lần vẽ đầu là lần đầu thấy lịch sử THẬT (đã hydrate), không phải lần effect
    // đầu với danh sách còn rỗng khi request còn pending.
    const firstPaint = !before.hydrated;
    before.messages = messages.length;
    before.tools = liveToolCount;
    before.confirms = confirmCount;
    before.hydrated = before.hydrated || !history.isPending;

    if (previousHeight.current !== null) {
      // Tải trang cũ hơn: giữ nguyên chiều cao khung cũ để cuộn không nhảy.
      element.scrollTop = element.scrollHeight - previousHeight.current;
      previousHeight.current = null;
      return;
    }
    if (manualScroll.current) {
      manualScroll.current = false;
      return;
    }
    if (!atBottom) {
      // Chỉ cuộn theo khi người dùng **đang ở đáy**. Họ cuộn lên để đọc lại một
      // đoạn nào đó thì tin nhắm mới (scheduler, tin chủ động) không được giật
      // màn hình họ ra khỏi chỗ đang đọc.
      scrollTween.current?.kill();
      return;
    }
    if (scrollTween.current?.isActive()) {
      // Đang trượt xuống vì khối mới: delta streaming tới nơi cũng để tween bám
      // đáy nốt — hai luồng cùng giành `scrollTop` mới chính là giật.
      return;
    }
    if (appended && !firstPaint) {
      // Khối mới (tin, thẻ tool, thẻ xác nhận) được CHÈN vào danh sách: trượt
      // mượt xuống thay vì nhảy tức thì.
      scrollTween.current?.kill();
      scrollTween.current = tweenToBottom(element);
    } else {
      // Delta streaming / lần vẽ đầu: bám tức thì để chữ chạy tới đâu thấy tới đó.
      element.scrollTop = element.scrollHeight;
    }
  }, [atBottom, scrollKey, history.isPending, messages.length, liveToolCount, confirmCount]);

  /** Người dùng tự cuộn: biết mình đang ở đâu để quyết định có cuộn theo không. */
  function onScroll() {
    const element = scrollRef.current;
    if (!element) return;
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    setAtBottom(distance < 48);
  }

  function scrollToBottom() {
    const element = scrollRef.current;
    if (!element) return;
    scrollTween.current?.kill();
    manualScroll.current = true;
    scrollTween.current = tweenToBottom(element, 0.35);
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
    // Gửi tin nghĩa là muốn xem **câu trả lời mới nhất**: đưa khung về đáy ngay,
    // bất kể đang đọc lại chỗ nào (khác với tin nhắm mới — tin nhắm không được
    // giật người dùng ra khỏi chỗ đang đọc). Bước nhảy thật do effect chạy sau
    // commit render (lúc thẻ "Đang suy nghĩ" đã nằm trong DOM để đo
    // `scrollHeight` cho đúng); ở đây chỉ cần chốt điều kiện `atBottom` cho lần
    // effect kế tiếp — `run.status` đổi từ `markSubmitting` sẽ đánh thức effect
    // ngay cả khi `atBottom` vốn đã đúng.
    setAtBottom(true);
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
              {messages.map((message) => {
                // `HistoryMessage` tự trả `null` cho vài trường hợp; bọc `<Enter>`
                // trực tiếp sẽ để lại div rỗng, làm hở dòng do `space-y-1` — nên
                // lọc trước khi bọc.
                const parsed = parseStoredMessage(message);
                const orphanTool = parsed?.role === "tool" && !toolResults.get(parsed.toolCallId ?? "");
                if (!parsed || orphanTool) return null;
                return (
                  <Enter key={message.id} when={isNew(message.id)} kind="rise">
                    <HistoryMessage dto={message} toolResults={toolResults} liveToolIds={liveToolIds} />
                  </Enter>
                );
              })}
              {Object.values(run?.liveTools ?? {})
                .filter((tool) => !toolResults.has(tool.id))
                .map((tool) => (
                  // Thẻ tool của run hiện tại luôn animate khi sinh ra: đó là
                  // lúc người dùng đang theo dõi, mỗi thẻ mới là một mốc tiến trình.
                  <Enter key={`${tool.runId}:${tool.id}`} kind="rise">
                    <ToolCard
                      name={tool.tool}
                      summary={tool.summary}
                      argsPreview={tool.argsPreview}
                      outputPreview={tool.outputPreview}
                      status={tool.status}
                    />
                  </Enter>
                ))}
              {run?.streamText ? (
                <Enter kind="rise">
                  <MessageRow author="assistant" headingId="stream-text">
                    <SafeMarkdown>{run.streamText}</SafeMarkdown>
                    {run.status === "running" || run.status === "stopping" ? <StreamingCaret /> : null}
                  </MessageRow>
                </Enter>
              ) : null}
              {confirms.map((confirm) => (
                <Enter key={confirm.confirm_id} kind="pop">
                  <ConfirmCard
                    confirm={confirm}
                    onDecision={(decision) => decide(confirm.confirm_id, decision)}
                    onExpire={expire}
                  />
                </Enter>
              ))}
            </div>
            {run?.status === "queued" ? (
              <Enter kind="pop" className="mt-4 flex justify-center">
                <QueuePill position={run.queuePosition} />
              </Enter>
            ) : null}
            {/* "Đang suy nghĩ" hiện cả khi vừa gửi (`submitting`) để có phản hồi
                tức thì, và khi run chạy mà chưa có chữ streaming. Khi chữ đã bắt
                đầu tới thì chính nó (kèm chỏ nháy) là tín hiệu sống — thêm ba
                chấm nữa chỉ là nhiễu. */}
            {run && (run.status === "submitting" || run.status === "running") && !run.streamText ? (
              <Enter kind="rise" className="mt-4 flex justify-center">
                <ThinkingIndicator />
              </Enter>
            ) : null}
            {run?.error ? (
              <Enter kind="shake" className="mt-4">
                <p className="rounded-lg bg-alert/10 p-3 text-sm text-alert" role="alert">
                  {run.error}
                </p>
              </Enter>
            ) : null}
          </div>
        </div>
        {/* Nút tròn nổi lên khi lệch khỏi đáy — theo mẫu Open WebUI. Chỉ hiện
            khi thực sự cần, không chiếm chỗ khi đang ở đáy. */}
        {atBottom ? null : (
          // `Enter` giữ tọa độ tuyệt đối; nút tự dịch tâm nên GSAP scale trên
          // wrapper không đụng tới `-translate-x-1/2` của nút.
          <Enter kind="pop" className="absolute bottom-4 left-1/2 z-10">
            <button
              type="button"
              onClick={scrollToBottom}
              aria-label={t("chat.scrollToBottom")}
              title={t("chat.scrollToBottom")}
              className="flex size-9 -translate-x-1/2 items-center justify-center rounded-full border border-rule bg-surface-raised text-ink shadow-md transition-colors hover:bg-surface-hover"
            >
              <ChevronDownIcon className="size-4" />
            </button>
          </Enter>
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
  const rootRef = useRef<HTMLDivElement>(null);

  // Stagger chào mừng: mascot → tiêu đề → hint → từng thẻ gợi ý. Chạy một lần
  // khi màn hình rỗng mở ra; không có nó thì mọi thứ bật sẵn cũng không sao.
  useGsapAnimate(() => {
    const root = rootRef.current;
    if (!root) return;
    gsap.from(Array.from(root.children), {
      opacity: 0,
      y: 16,
      duration: 0.45,
      ease: "power2.out",
      stagger: 0.08,
      clearProps: "opacity,transform",
    });
    gsap.from(root.querySelectorAll("li"), {
      opacity: 0,
      y: 12,
      duration: 0.4,
      ease: "power2.out",
      stagger: 0.06,
      delay: 0.2,
      clearProps: "opacity,transform",
    });
  }, []);

  const suggestions = [
    t("chat.suggestion.plan"),
    t("chat.suggestion.explore"),
    t("chat.suggestion.memory"),
    t("chat.suggestion.summarize"),
  ];
  return (
    <div ref={rootRef} className="flex flex-col items-center px-2 py-16 text-center">
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
  const dotRef = useRef<HTMLSpanElement>(null);

  // Chấm "đã kết nối" đập nhẹ — sống khác với chấm đứng yên báo mất kết nối.
  useGsapAnimate(() => {
    const dot = dotRef.current;
    if (!dot || status !== "connected") return;
    gsap.to(dot, { scale: 1.4, opacity: 0.5, duration: 0.7, ease: "sine.inOut", yoyo: true, repeat: -1 });
  }, [status]);

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
      <span ref={dotRef} className="h-2 w-2 rounded-full bg-current" aria-hidden="true" />
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
