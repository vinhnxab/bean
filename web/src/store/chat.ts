import { create } from "zustand";

import type { PendingConfirm, ServerMsg } from "@/api/bindings";

export type RunStatus = "idle" | "submitting" | "queued" | "running" | "stopping";
export type ConfirmState = PendingConfirm & {
  receivedAt: number;
  resolution: "pending" | "submitting" | "allowed" | "denied" | "expired";
};

export type LiveTool = {
  id: string;
  runId: string;
  tool: string;
  summary: string;
  argsPreview: string;
  status: "running" | "ok" | "error";
  outputPreview: string;
};

export type SessionRunState = {
  runId: string | null;
  status: RunStatus;
  queuePosition: number | null;
  streamText: string;
  liveTools: Record<string, LiveTool>;
  error: string | null;
};

export type RefreshEffect = {
  sessionId: number;
  messageId: number | null;
  runId: string | null;
} | null;

type ChatState = {
  runsBySession: Record<number, SessionRunState>;
  confirmsById: Record<string, ConfirmState>;
  seenEventKeys: Record<string, true>;
  markSubmitting: (sessionId: number) => void;
  markStopping: (sessionId: number) => void;
  applySync: (message: Extract<ServerMsg, { type: "sync" }>, reconcile: boolean) => void;
  applyServerMessage: (message: Exclude<ServerMsg, { type: "sync" | "pong" }>) => RefreshEffect;
  setConfirmSubmitting: (confirmId: string, submitting: boolean) => void;
  expireConfirm: (confirmId: string) => void;
  reset: () => void;
};

function idleRun(): SessionRunState {
  return {
    runId: null,
    status: "idle",
    queuePosition: null,
    streamText: "",
    liveTools: {},
    error: null,
  };
}

function runFor(state: ChatState, sessionId: number, update: Partial<SessionRunState> = {}): SessionRunState {
  return { ...(state.runsBySession[sessionId] ?? idleRun()), ...update };
}

function eventKey(message: Exclude<ServerMsg, { type: "sync" | "pong" }>): string | null {
  if (message.type === "text_delta") return null;
  switch (message.type) {
    case "queued":
      return `${message.type}:${message.run_id}`;
    case "tool_start":
    case "tool_end":
      return `${message.type}:${message.run_id}:${message.id}`;
    case "final":
      return `${message.type}:${message.run_id}`;
    case "notification":
      return `${message.type}:${message.session_id}:${message.message_id}`;
    case "error":
      return `${message.type}:${message.session_id ?? "global"}:${message.run_id ?? "none"}`;
    case "confirm_request":
      return `${message.type}:${message.confirm_id}`;
    case "confirm_resolved":
      return `${message.type}:${message.confirm_id}`;
    case "text":
      return `${message.type}:${message.run_id}:${message.text}`;
  }
}

/** Chỉ giữ 500 khóa event gần nhất để chống duplicate mà không phình vô hạn. */
function rememberEvent(seen: Record<string, true>, key: string): Record<string, true> {
  if (seen[key]) return seen;
  const next = { ...seen, [key]: true as const };
  const keys = Object.keys(next);
  if (keys.length > 500) {
    for (const oldKey of keys.slice(0, keys.length - 500)) delete next[oldKey];
  }
  return next;
}

function resolutionFromOutcome(outcome: string): ConfirmState["resolution"] {
  if (outcome === "allowed") return "allowed";
  if (outcome === "denied") return "denied";
  return "expired";
}

const initialRuns: Record<number, SessionRunState> = {};
const initialConfirms: Record<string, ConfirmState> = {};

export const useChatStore = create<ChatState>((set, get) => ({
  runsBySession: initialRuns,
  confirmsById: initialConfirms,
  seenEventKeys: {},

  markSubmitting: (sessionId) =>
    set((state) => ({
      runsBySession: {
        ...state.runsBySession,
        [sessionId]: runFor(state, sessionId, {
          runId: null,
          status: "submitting",
          queuePosition: null,
          streamText: "",
          liveTools: {},
          error: null,
        }),
      },
    })),

  markStopping: (sessionId) =>
    set((state) => ({
      runsBySession: {
        ...state.runsBySession,
        [sessionId]: runFor(state, sessionId, { status: "stopping" }),
      },
    })),

  applySync: (message, reconcile) =>
    set((state) => {
      const runs = { ...state.runsBySession };
      const activeRunIds = new Set(message.running.map((item) => item.run_id));
      for (const [sessionId, run] of Object.entries(runs)) {
        if (run.runId && !activeRunIds.has(run.runId) && run.status !== "idle")
          runs[Number(sessionId)] = idleRun();
      }
      for (const item of message.running) {
        const previous = runs[item.session_id];
        runs[item.session_id] = {
          ...(previous ?? idleRun()),
          runId: item.run_id,
          status: "running",
          streamText: previous?.runId === item.run_id ? previous.streamText : "",
          liveTools: previous?.runId === item.run_id ? previous.liveTools : {},
          error: null,
        };
      }
      const confirms = { ...state.confirmsById };
      if (reconcile) {
        const pendingIds = new Set(message.pending_confirms.map((item) => item.confirm_id));
        for (const [confirmId, confirm] of Object.entries(confirms)) {
          if (confirm.resolution === "pending" && !pendingIds.has(confirmId)) delete confirms[confirmId];
        }
      }
      for (const pending of message.pending_confirms) {
        const previous = confirms[pending.confirm_id];
        confirms[pending.confirm_id] = {
          ...pending,
          receivedAt: previous?.receivedAt ?? Date.now(),
          resolution: previous?.resolution === "pending" ? "pending" : (previous?.resolution ?? "pending"),
        };
      }
      return { runsBySession: runs, confirmsById: confirms };
    }),

  applyServerMessage: (message) => {
    const key = message.type === "text_delta" ? null : eventKey(message);
    if (key && get().seenEventKeys[key]) return null;
    let effect: RefreshEffect = null;
    set((state) => {
      const seenEventKeys = key ? rememberEvent(state.seenEventKeys, key) : state.seenEventKeys;
      const runs = { ...state.runsBySession };
      const confirms = { ...state.confirmsById };
      switch (message.type) {
        case "queued":
          runs[message.session_id] = runFor(state, message.session_id, {
            runId: message.run_id,
            status: "queued",
            queuePosition: message.position,
            error: null,
          });
          break;
        case "text":
          runs[message.session_id] = runFor(state, message.session_id, {
            runId: message.run_id,
            status: "running",
            queuePosition: null,
            streamText: message.text,
          });
          effect = { sessionId: message.session_id, messageId: null, runId: message.run_id };
          break;
        case "text_delta": {
          const run = runFor(state, message.session_id);
          if (run.runId !== null && run.runId !== message.run_id) break;
          runs[message.session_id] = {
            ...run,
            runId: message.run_id,
            status: "running",
            queuePosition: null,
            streamText: message.reset ? message.text : run.streamText + message.text,
          };
          break;
        }
        case "tool_start": {
          const run = runFor(state, message.session_id, {
            runId: message.run_id,
            status: "running",
            queuePosition: null,
          });
          runs[message.session_id] = {
            ...run,
            liveTools: {
              ...run.liveTools,
              [message.id]: {
                id: message.id,
                runId: message.run_id,
                tool: message.tool,
                summary: message.summary,
                argsPreview: message.args_preview,
                status: "running",
                outputPreview: "",
              },
            },
          };
          effect = { sessionId: message.session_id, messageId: null, runId: message.run_id };
          break;
        }
        case "tool_end": {
          const run = runFor(state, message.session_id);
          const tool = run.liveTools[message.id];
          runs[message.session_id] = {
            ...run,
            status: "running",
            liveTools: tool
              ? {
                  ...run.liveTools,
                  [message.id]: {
                    ...tool,
                    status: message.ok ? "ok" : "error",
                    outputPreview: message.output_preview,
                  },
                }
              : run.liveTools,
          };
          effect = { sessionId: message.session_id, messageId: null, runId: message.run_id };
          break;
        }
        case "confirm_request":
          runs[message.session_id] = runFor(state, message.session_id, {
            runId: message.run_id,
            status: "running",
            queuePosition: null,
          });
          confirms[message.confirm_id] = { ...message, receivedAt: Date.now(), resolution: "pending" };
          break;
        case "confirm_resolved": {
          const confirm = confirms[message.confirm_id];
          if (confirm)
            confirms[message.confirm_id] = { ...confirm, resolution: resolutionFromOutcome(message.outcome) };
          break;
        }
        case "final":
          runs[message.session_id] = {
            ...runFor(state, message.session_id, { status: "idle" }),
            runId: null,
            queuePosition: null,
            streamText: "",
            liveTools: {},
          };
          effect = { sessionId: message.session_id, messageId: message.message_id, runId: message.run_id };
          break;
        case "notification":
          effect = { sessionId: message.session_id, messageId: message.message_id, runId: null };
          break;
        case "error": {
          const sessionId = message.session_id;
          if (sessionId !== null) {
            runs[sessionId] = {
              ...runFor(state, sessionId, { error: message.message, status: "idle" }),
              runId: null,
              queuePosition: null,
              streamText: "",
              liveTools: {},
            };
          }
          break;
        }
      }
      return { runsBySession: runs, confirmsById: confirms, seenEventKeys };
    });
    return effect;
  },

  setConfirmSubmitting: (confirmId, submitting) =>
    set((state) => {
      const confirm = state.confirmsById[confirmId];
      if (!confirm || (confirm.resolution !== "pending" && !submitting)) return state;
      return {
        confirmsById: {
          ...state.confirmsById,
          [confirmId]: { ...confirm, resolution: submitting ? "submitting" : "pending" },
        },
      };
    }),

  expireConfirm: (confirmId) =>
    set((state) => {
      const confirm = state.confirmsById[confirmId];
      if (confirm?.resolution !== "pending") return state;
      return {
        confirmsById: {
          ...state.confirmsById,
          [confirmId]: { ...confirm, resolution: "expired" },
        },
      };
    }),

  reset: () => set({ runsBySession: {}, confirmsById: {}, seenEventKeys: {} }),
}));
