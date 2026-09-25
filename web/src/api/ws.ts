import type { ClientMsg, ServerMsg } from "@/api/bindings";

const OPEN = 1;
const CONNECTING = 0;
const PING_INTERVAL_MS = 20_000;
const RECONNECT_BASE_MS = 500;
const RECONNECT_MAX_MS = 30_000;

export type SocketStatus = "idle" | "connecting" | "connected" | "reconnecting";
export type SyncReason = "initial" | "resync" | "reconnect";

export type ChatSocketOptions = {
  socketFactory: (url: string) => WebSocket;
  onMessage: (message: ServerMsg) => void;
  onSync: (message: Extract<ServerMsg, { type: "sync" }>, reason: SyncReason) => void;
  onStatus?: (status: SocketStatus) => void;
  random?: () => number;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasNumber(value: Record<string, unknown>, key: string): boolean {
  return typeof value[key] === "number" && Number.isFinite(value[key]);
}

function hasString(value: Record<string, unknown>, key: string): boolean {
  return typeof value[key] === "string";
}

function hasNullableString(value: Record<string, unknown>, key: string): boolean {
  return value[key] === null || typeof value[key] === "string";
}

function isPendingConfirm(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return (
    hasString(value, "confirm_id") &&
    hasNumber(value, "session_id") &&
    hasString(value, "run_id") &&
    hasString(value, "prompt") &&
    (value.risk === "safe" || value.risk === "confirm" || value.risk === "dangerous") &&
    typeof value.allow_session_option === "boolean" &&
    hasNumber(value, "timeout_seconds")
  );
}

function isRunningInfo(value: unknown): boolean {
  return isRecord(value) && hasNumber(value, "session_id") && hasString(value, "run_id");
}

/** Kiểm tra wire JSON ở biên trước khi đưa event vào store. */
export function parseServerMessage(raw: unknown): ServerMsg | null {
  if (!isRecord(raw) || typeof raw.type !== "string") return null;
  const valid = (() => {
    switch (raw.type) {
      case "sync":
        return (
          Array.isArray(raw.running) &&
          raw.running.every(isRunningInfo) &&
          Array.isArray(raw.pending_confirms) &&
          raw.pending_confirms.every(isPendingConfirm)
        );
      case "queued":
        return hasNumber(raw, "session_id") && hasString(raw, "run_id") && hasNumber(raw, "position");
      case "text":
      case "text_delta":
        return (
          hasNumber(raw, "session_id") &&
          hasString(raw, "run_id") &&
          hasString(raw, "text") &&
          (raw.type === "text" ||
            (hasNumber(raw, "index") &&
              typeof raw.index === "number" &&
              Number.isInteger(raw.index) &&
              raw.index >= 0 &&
              typeof raw.reset === "boolean"))
        );
      case "tool_start":
        return (
          hasNumber(raw, "session_id") &&
          hasString(raw, "run_id") &&
          hasString(raw, "id") &&
          hasString(raw, "tool") &&
          hasString(raw, "summary") &&
          hasString(raw, "args_preview")
        );
      case "tool_end":
        return (
          hasNumber(raw, "session_id") &&
          hasString(raw, "run_id") &&
          hasString(raw, "id") &&
          typeof raw.ok === "boolean" &&
          hasString(raw, "output_preview")
        );
      case "confirm_request":
        return isPendingConfirm(raw);
      case "confirm_resolved":
        return hasString(raw, "confirm_id") && hasString(raw, "outcome");
      case "final":
        return hasNumber(raw, "session_id") && hasString(raw, "run_id") && hasNumber(raw, "message_id");
      case "notification":
        return hasNumber(raw, "session_id") && hasNumber(raw, "message_id");
      case "error":
        return (
          hasNullableString(raw, "session_id") &&
          hasNullableString(raw, "run_id") &&
          hasString(raw, "code") &&
          hasString(raw, "message")
        );
      case "pong":
        return true;
      default:
        return false;
    }
  })();
  return valid ? (raw as unknown as ServerMsg) : null;
}

/** WebSocket dùng chung cho mọi hội thoại; không sở hữu run và không tự huỷ run. */
export class ChatSocket {
  private readonly socketFactory: (url: string) => WebSocket;
  private readonly onMessage: (message: ServerMsg) => void;
  private readonly onSync: ChatSocketOptions["onSync"];
  private readonly onStatus: (status: SocketStatus) => void;
  private readonly random: () => number;
  private socket: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private reconnectAttempt = 0;
  private connectedOnce = false;
  private awaitingSyncAfterReconnect = false;
  private started = false;

  constructor(options: ChatSocketOptions) {
    this.socketFactory = options.socketFactory;
    this.onMessage = options.onMessage;
    this.onSync = options.onSync;
    this.onStatus = options.onStatus ?? (() => undefined);
    this.random = options.random ?? Math.random;
  }

  start(): void {
    if (this.started) return;
    this.started = true;
    this.connect();
  }

  send(message: ClientMsg): boolean {
    if (!this.socket || this.socket.readyState !== OPEN) return false;
    try {
      this.socket.send(JSON.stringify(message));
      return true;
    } catch {
      return false;
    }
  }

  stop(): void {
    this.started = false;
    this.clearTimers();
    const socket = this.socket;
    this.socket = null;
    if (!socket) return;
    socket.onopen = null;
    socket.onmessage = null;
    socket.onerror = null;
    socket.onclose = null;
    if (socket.readyState === OPEN || socket.readyState === CONNECTING) {
      socket.close(1000, "client closed");
    }
    this.onStatus("idle");
  }

  private connect(): void {
    if (!this.started) return;
    this.onStatus(this.reconnectAttempt === 0 ? "connecting" : "reconnecting");
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${protocol}//${window.location.host}/api/ws`;
    const socket = this.socketFactory(url);
    this.socket = socket;

    socket.onopen = () => {
      if (this.socket !== socket || !this.started) return;
      this.onStatus("connected");
      this.startPing();
    };
    socket.onmessage = (event: MessageEvent<unknown>) => {
      if (this.socket !== socket || typeof event.data !== "string") return;
      let parsed: unknown;
      try {
        parsed = JSON.parse(event.data) as unknown;
      } catch {
        return;
      }
      const message = parseServerMessage(parsed);
      if (!message) return;
      if (message.type === "sync") {
        const reason: SyncReason = this.awaitingSyncAfterReconnect
          ? "reconnect"
          : !this.connectedOnce
            ? "initial"
            : "resync";
        this.connectedOnce = true;
        this.awaitingSyncAfterReconnect = false;
        this.reconnectAttempt = 0;
        this.onSync(message, reason);
      } else {
        this.onMessage(message);
      }
    };
    socket.onerror = () => socket.close();
    socket.onclose = () => {
      if (this.socket !== socket || !this.started) return;
      this.socket = null;
      this.clearPing();
      this.scheduleReconnect();
    };
  }

  private startPing(): void {
    this.clearPing();
    this.pingTimer = setInterval(() => {
      this.send({ type: "ping" });
    }, PING_INTERVAL_MS);
  }

  private scheduleReconnect(): void {
    if (!this.started) return;
    this.onStatus("reconnecting");
    this.awaitingSyncAfterReconnect = true;
    const exponential = Math.min(RECONNECT_MAX_MS, RECONNECT_BASE_MS * 2 ** this.reconnectAttempt);
    const jitter = Math.floor(exponential * 0.25 * this.random());
    this.reconnectAttempt += 1;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, exponential + jitter);
  }

  private clearPing(): void {
    if (this.pingTimer !== null) clearInterval(this.pingTimer);
    this.pingTimer = null;
  }

  private clearTimers(): void {
    this.clearPing();
    if (this.reconnectTimer !== null) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
  }
}
