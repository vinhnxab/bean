import { useQueryClient } from "@tanstack/react-query";
import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { DecisionDto, ServerMsg } from "@/api/bindings";
import { ChatSocket, type SocketStatus } from "@/api/ws";
import { queryKeys } from "@/features/auth/queries";
import { useChatStore } from "@/store/chat";

type RealtimeValue = {
  status: SocketStatus;
  sendStart: (sessionId: number, text: string) => boolean;
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
        }
      },
    });
    setSocket(instance);
    instance.start();
    return () => {
      instance.stop();
      setSocket(null);
    };
  }, [applyServerMessage, applySync, queryClient]);

  const sendStart = useCallback(
    (sessionId: number, text: string) => {
      if (!socket?.send({ type: "start", session_id: sessionId, text })) return false;
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
    [sendCancel, sendConfirm, sendStart, status],
  );
  return <RealtimeContext.Provider value={value}>{children}</RealtimeContext.Provider>;
}

export function useRealtime(): RealtimeValue {
  const value = useContext(RealtimeContext);
  if (!value) throw new Error("useRealtime phải được dùng trong RealtimeProvider");
  return value;
}
