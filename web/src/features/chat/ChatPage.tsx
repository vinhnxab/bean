import gsap from "gsap";
import { ChevronDownIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useParams } from "react-router";
import type { DecisionDto, MessageDto } from "@/api/bindings";
import { Composer } from "@/components/chat/Composer";
import { ConfirmCard } from "@/components/chat/ConfirmCard";
import { MessageRow } from "@/components/chat/MessageRow";
import { PlanCard } from "@/components/chat/PlanCard";
import { QueuePill, ThinkingIndicator } from "@/components/chat/RunStatus";
import { SafeMarkdown } from "@/components/markdown/SafeMarkdown";
import {
  deduplicateMessages,
  isWorkMessage,
  mergeConfirmFlow,
  parseStoredMessage,
  type StoredMessage,
} from "@/features/chat/messages";
import { useSessionMessages, useVoiceStatus } from "@/features/chat/queries";
import { useRealtime } from "@/features/chat/RealtimeProvider";
import { useI18n } from "@/i18n";
import type { MessageKey } from "@/i18n/vi";
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
  const voice = useVoiceStatus();
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
  const confirms = useMemo(
    () => Object.values(allConfirms).filter((item) => item.session_id === sessionId),
    [allConfirms, sessionId],
  );

  const confirmCount = confirms.length;

  // Thẻ xác nhận nằm đúng dòng thời gian thay vì luôn ghim ở cuối: khi `final`
  // nạp lại lịch sử, câu trả lời cuối không còn chen lên trước thẻ đã duyệt.
  const flow = useMemo(() => mergeConfirmFlow(messages, confirms), [messages, confirms]);

  // --- Hoạt ảnh "tin mới" --------------------------------------------------
  // Lịch sử sẵn có khi mở phiên KHÔNG animate: mở một phiên dài mà mỗi tin lại
  // trượt vào một lần là một cascade hỗn loạn. Chỉ tin xuất hiện SAU lần hydrate
  // đầu mới animate (`isNew`). Component này không unmount khi đổi hội thoại
  // (cùng vị trí route), nên state theo phiên phải reset ngay tại render.
  const seeded = useRef<SeedState | null>(null);
  // Khối lượng nội dung để effect cuộn biết có thứ mới được CHÈN vào.
  const volume = useRef({ messages: 0, confirms: 0, hydrated: false });
  const scrollTween = useRef<ScrollTween | null>(null);
  // Đặt khi người dùng bấm nút xuống đáy: effect kế tiếp phải nhả tay để tween
  // trượt nốt, không đè xuống nhảy tức thì.
  const manualScroll = useRef(false);
  if (seeded.current === null || seeded.current.sessionId !== sessionId) {
    seeded.current = seedTracker(sessionId);
    volume.current = { messages: 0, confirms: 0, hydrated: false };
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
  // lời mới nhất", nên thẻ suy nghĩ/xếp hàng phải nằm trong khung nhìn ngay sau
  // commit render. Text của run không render nữa (chỉ hiện kết quả cuối cùng
  // lấy từ DB khi `final`), nên không cần đưa vào khóa.
  const scrollKey = `${messages.length}:${confirmCount}:${run?.status ?? "idle"}:${run?.queuePosition ?? ""}`;

  useEffect(() => {
    void scrollKey;
    const element = scrollRef.current;
    if (!element) return;
    const before = volume.current;
    const appended = messages.length > before.messages || confirmCount > before.confirms;
    // Lần vẽ đầu là lần đầu thấy lịch sử THẬT (đã hydrate), không phải lần effect
    // đầu với danh sách còn rỗng khi request còn pending.
    const firstPaint = !before.hydrated;
    before.messages = messages.length;
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
  }, [atBottom, scrollKey, history.isPending, messages.length, confirmCount]);

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

  /**
   * Gửi một lượt voice (M27): văn bản đến từ mic được đánh dấu `voice = true` để
   * Router giới hạn tool ở mức `Safe` và yêu cầu model trả lời một câu ngắn cho TTS.
   */
  const sendVoice = useCallback(
    (recognized: string) => {
      const value = recognized.trim();
      if (!value || status !== "connected") {
        setSendError(true);
        return;
      }
      if (!sendStart(sessionId, value, true)) {
        setSendError(true);
        return;
      }
      setSendError(false);
      setAtBottom(true);
    },
    [sendStart, sessionId, status],
  );

  function loadOlder() {
    const element = scrollRef.current;
    if (element) previousHeight.current = element.scrollHeight;
    void history.fetchNextPage();
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Vùng cuộn nằm gọn trong đảo nội dung: nav đã là đảo riêng phía trên
          nên không còn mẹo kéo vùng cuộn chui dưới nav — tin nhắn bị bo góc
          của đảo cắt tại mép trên, đúng như Telegram Web cắt danh sách tin. */}
      <div className="relative min-h-0 flex-1">
        <div
          ref={scrollRef}
          onScroll={onScroll}
          className="h-full overflow-y-auto px-2 pb-4 pt-4 md:px-4"
          aria-live="polite"
          aria-relevant="additions text"
        >
          {/* Cột đọc về lại bề rộng vừa phải (`max-w-3xl`, căn giữa): dòng dài
              quá ~70–80 ký tự làm mắt mất dấu khi xuống dòng; khối code dài vẫn
              cuộn ngang bên trong khối của nó, không phụ thuộc bề rộng cột. */}
          <div className="mx-auto max-w-3xl">
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
            <div className="space-y-6">
              {flow.map((item) => {
                if (item.kind === "confirm") {
                  const confirm = item.confirm;
                  return (
                    <Enter key={confirm.confirm_id} kind="pop">
                      <ConfirmCard
                        confirm={confirm}
                        onDecision={(decision) => decide(confirm.confirm_id, decision)}
                        onExpire={expire}
                      />
                    </Enter>
                  );
                }
                // `HistoryMessage` tự trả `null` cho tin quá trình (lời gọi tool,
                // kết quả tool); lọc trước khi bọc — bọc `<Enter>` quanh `null` sẽ
                // để lại div rỗng làm hở dòng do `space-y-6`.
                const parsed = parseStoredMessage(item.message);
                if (!parsed || isWorkMessage(parsed)) return null;
                return (
                  <Enter key={item.message.id} when={isNew(item.message.id)} kind="rise">
                    <HistoryMessage dto={item.message} />
                  </Enter>
                );
              })}
            </div>
            {run?.status === "queued" ? (
              <Enter kind="pop" className="mt-4 flex justify-center">
                <QueuePill position={run.queuePosition} />
              </Enter>
            ) : null}
            {/* Quá trình trung gian (text suy nghĩ, tool đang chạy) không hiển thị —
                chỉ pill "Đang suy nghĩ" với đồng hồ trôi. Hiện ngay từ lúc gửi
                (`submitting`) để có phản hồi tức thì; khi `stopping` đổi nhãn thành
                "Đang dừng…". Kết quả cuối cùng xuất hiện khi `final` nạp lại lịch sử. */}
            {run && (run.status === "submitting" || run.status === "running" || run.status === "stopping") ? (
              <div className="mt-4 space-y-3">
                {/* Kế hoạch bước (D25) hiện ngay trên pill "Đang suy nghĩ" — giúp
                    việc dài không lạc hướng; biến mất cùng run khi kết thúc. */}
                {run.plan.length > 0 ? (
                  <Enter kind="rise">
                    <PlanCard steps={run.plan} />
                  </Enter>
                ) : null}
                <Enter kind="rise" className="flex justify-center">
                  <ThinkingIndicator startedAt={run.startedAt} stopping={run.status === "stopping"} />
                </Enter>
              </div>
            ) : null}
            {run && (run.error || run.errorCode === "interrupted") ? (
              <Enter kind="shake" className="mt-4">
                <p className="rounded-lg bg-alert/10 p-3 text-sm text-alert" role="alert">
                  {run.error ?? t("chat.interrupted")}
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
        voiceEnabled={voice.data?.enabled === true}
        onVoice={sendVoice}
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
/**
 * Lệnh slash có sẵn của lõi — lõi xử lý ngay, không tốn một lượt LLM. Bấm một
 * lệnh bơm thẳng vào ô soạn: người dùng Enter là gửi. `descKey` làm nhãn trợ
 * năng (ticker chỉ hiện lệnh, mô tả không kèm để giữ nhịp chạy).
 */
const COMMANDS: { cmd: string; descKey: MessageKey }[] = [
  { cmd: "/new", descKey: "chat.cmd.new" },
  { cmd: "/memory", descKey: "chat.cmd.memory" },
  { cmd: "/skills", descKey: "chat.cmd.skills" },
  { cmd: "/tasks", descKey: "chat.cmd.tasks" },
  { cmd: "/model", descKey: "chat.cmd.model" },
  { cmd: "/stop", descKey: "chat.cmd.stop" },
];

/**
 * Dải lệnh chạy ngang vô hạn kiểu “logo wall” (hàng thương hiệu đối tác):
 * track chứa HAI bản copy giống hệt nhau, CSS animation `translateX(-50%)`
 * (class `.cmd-ticker-*` ở index.css) cho vòng lặp liền mạch — không dùng thẻ
 * `<marquee>` gốc vì nó chờ toàn bộ nội dung đi hết mới quay lại, tạo khoảng
 * trống thấy rõ. Bản copy thứ hai `aria-hidden` + `tabIndex={-1}`: chỉ bản
 * đầu vào được cây trợ năng và tab. Rê chuột dừng để đọc (CSS hover).
 */
function CommandTicker({ onPick }: { onPick: (cmd: string) => void }) {
  const { t } = useI18n();

  function row(copy: number) {
    const hidden = copy > 0;
    return (
      <ul
        className="cmd-ticker-row"
        aria-hidden={hidden || undefined}
        aria-label={copy === 0 ? t("chat.suggestionsLabel") : undefined}
      >
        {COMMANDS.map(({ cmd, descKey }) => (
          <li key={cmd}>
            <button
              type="button"
              onClick={() => onPick(cmd)}
              tabIndex={hidden ? -1 : undefined}
              aria-label={t(descKey)}
              className="cmd-item"
            >
              <span className="cmd-prompt" aria-hidden="true">
                $
              </span>
              <span className="cmd-text">{cmd}</span>
            </button>
          </li>
        ))}
        <li aria-hidden="true" className="cmd-cursor">
          ▍
        </li>
      </ul>
    );
  }

  return (
    <div className="cmd-ticker mt-8 w-full max-w-2xl">
      <div className="cmd-ticker-track">
        {row(0)}
        {row(1)}
      </div>
    </div>
  );
}

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

  return (
    <div ref={rootRef} className="flex flex-col items-center px-2 py-16 text-center">
      {/* Hình mascot thật (`public/bean-ico.png`) thay cho mascot vector: giữ
          `h-16 w-auto` vì ảnh gốc 352×192 là khung ngang, `rounded-xl` hoà với
          bo góc kính của các thẻ bên dưới. */}
      <img
        src="/bean-ico.png"
        alt={t("app.title")}
        width={352}
        height={192}
        decoding="async"
        className="mb-4 h-16 w-auto rounded-xl"
      />
      <h2 className="text-2xl font-semibold tracking-tight">
        {t("chat.greeting")} <span className="text-brand">Bean</span>
      </h2>
      <p className="mt-2 text-sm text-muted-foreground">{t("chat.emptyHint")}</p>
      <CommandTicker onPick={onPick} />
    </div>
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

function HistoryMessage({ dto }: { dto: MessageDto }) {
  const parsed = parseStoredMessage(dto);
  if (!parsed || isWorkMessage(parsed)) return null;
  if (parsed.role === "tool") {
    // Tới đây chỉ còn kết quả tool mang ảnh (M26 — `browser_screenshot`): ảnh là
    // nội dung người dùng cần xem, không phải nhiễu quá trình, nên hiện thẳng
    // thay vì bọc thẻ tool. `mediaType`/`data` đã được kiệm ở biên
    // (`parseImage`), `src` luôn là URI `data:` — điều CSP `img-src 'self' data:`
    // chốt lại thành lớp thứ hai. Không dùng `dangerouslySetInnerHTML`.
    const image = parsed.image;
    if (!image) return null;
    return (
      <MessageRow author="assistant" headingId={`tool-img-${dto.id}`}>
        <img
          src={`data:${image.mediaType};base64,${image.data}`}
          alt={parsed.text || "browser_screenshot"}
          className="max-h-96 w-full max-w-xl rounded-lg border border-border object-contain"
        />
      </MessageRow>
    );
  }
  return <MessageBubble messageRole={parsed.role}>{parsed.text}</MessageBubble>;
}

function MessageBubble({ messageRole, children }: { messageRole: StoredMessage["role"]; children: string }) {
  // Tin lịch sử đi qua `MessageRow` để có tên người gửi + nút sao chép khi rê.
  // `role="tool"` không bao giờ tới đây — tin quá trình bị chặn ở `isWorkMessage`.
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
