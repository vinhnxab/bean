import { useQueryClient } from "@tanstack/react-query";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { DecisionDto, ServerMsg } from "@/api/bindings";
import { api } from "@/api/client";
import { ChatSocket, type SocketStatus } from "@/api/ws";
import { queryKeys } from "@/features/auth/queries";
import { playBase64Audio } from "@/lib/audio";
import { useChatStore } from "@/store/chat";

type RealtimeValue = {
  status: SocketStatus;
  /**
   * Gửi một tin bắt đầu run.
   *
   * `voice = true` khi nội dung đến từ nút mic push-to-talk (M27): Router giới hạn
   * lượt chỉ chạy tool `Safe` và chèn hướng dẫn trả lời một câu ngắn để TTS đọc.
   */
  sendStart: (sessionId: number, text: string, voice?: boolean) => boolean;
  sendCancel: (sessionId: number) => boolean;
  sendConfirm: (confirmId: string, decision: DecisionDto) => boolean;
};

const RealtimeContext = createContext<RealtimeValue | null>(null);

/** Một socket cho toàn app; đóng/reconnect không gắn với một run cụ thể. */
export function RealtimeProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const [status, setStatus] = useState<SocketStatus>("idle");
  const [socket, setSocket] = useState<ChatSocket | null>(null);
  const markSubmitting = useChatStore((state) => state.markSubmitting);
  const markStopping = useChatStore((state) => state.markStopping);
  const applySync = useChatStore((state) => state.applySync);
  const applyServerMessage = useChatStore((state) => state.applyServerMessage);
  const setConfirmSubmitting = useChatStore((state) => state.setConfirmSubmitting);

  // Session đã phát một lượt `voice = true` và đang chờ `final` để đọc thành tiếng.
  // Là `ref` vì không tham gia render — chỉ là bộ nhớ giữa `sendStart` và sự kiện
  // `final` (tới sau đó, ở vòng select của socket).
  const voiceTurns = useRef<Set<number>>(new Set());

  /**
   * Đọc câu trả lời cuối bằng TTS (M27). Lỗi (voice tắt, mạng, autoplay bị chặn)
   * **không** được làm hỏng UI — người dùng vẫn đọc được tin nhắn trong khung chat.
   */
  const speakReply = useCallback(async (text: string) => {
    const spoken = text.trim();
    if (!spoken) return;
    try {
      const result = await api.synthesize({ text: spoken });
      await playBase64Audio(result.audio_base64, result.mime);
    } catch {
      // Im lặng có chủ đích.
    }
  }, []);

  useEffect(() => {
    const instance = new ChatSocket({
      socketFactory: (url) => new WebSocket(url),
      onStatus: setStatus,
      onSync: (message, reason) => {
        applySync(message, reason !== "initial");
        if (reason === "reconnect" || reason === "resync") {
          void queryClient.invalidateQueries({ queryKey: ["messages"] });
        }
      },
      onMessage: (message: ServerMsg) => {
        if (message.type === "pong" || message.type === "sync") return;
        const effect = applyServerMessage(message);
        if (effect) {
          void queryClient.invalidateQueries({ queryKey: queryKeys.messages(effect.sessionId) });
          if (message.type === "final" || message.type === "notification") {
            void queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
          }
          if (message.type === "notification") {
            void queryClient.invalidateQueries({ queryKey: queryKeys.skillDrafts });
          }
          // Lượt voice: đọc câu trả lời cuối bằng TTS. Chỉ đọc cho session vừa phát
          // `voice = true`, nên thông báo nền của session khác không bị đọc oan.
          if (message.type === "final" && voiceTurns.current.has(message.session_id)) {
            voiceTurns.current.delete(message.session_id);
            void speakReply(message.text);
          }
        }
      },
    });
    setSocket(instance);
    instance.start();
    return () => {
      instance.stop();
      setSocket(null);
    };
  }, [applyServerMessage, applySync, queryClient, speakReply]);

  const sendStart = useCallback(
    (sessionId: number, text: string, voice = false) => {
      if (!socket?.send({ type: "start", session_id: sessionId, text, voice })) return false;
      // Nhớ session này để đọc câu trả lời khi `final` tới (xem `onMessage`).
      if (voice) voiceTurns.current.add(sessionId);
      markSubmitting(sessionId);
      return true;
    },
    [markSubmitting, socket],
  );
  const sendCancel = useCallback(
    (sessionId: number) => {
      if (!socket?.send({ type: "cancel", session_id: sessionId })) return false;
      markStopping(sessionId);
      return true;
    },
    [markStopping, socket],
  );
  const sendConfirm = useCallback(
    (confirmId: string, decision: DecisionDto) => {
      if (!socket?.send({ type: "confirm", confirm_id: confirmId, decision })) return false;
      setConfirmSubmitting(confirmId, true);
      return true;
    },
    [setConfirmSubmitting, socket],
  );

  const value = useMemo(
    () => ({ status, sendStart, sendCancel, sendConfirm }),
    [status, sendStart, sendCancel, sendConfirm],
  );
  return <RealtimeContext.Provider value={value}>{children}</RealtimeContext.Provider>;
}

export function useRealtime(): RealtimeValue {
  const value = useContext(RealtimeContext);
  if (!value) throw new Error("useRealtime phải được dùng trong RealtimeProvider");
  return value;
}
