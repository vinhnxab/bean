import { afterEach, describe, expect, it, vi } from "vitest";
import type { ServerMsg } from "@/api/bindings";
import { ChatSocket, parseServerMessage } from "@/api/ws";

class FakeSocket {
  static instances: FakeSocket[] = [];
  readyState = 0;
  sent: string[] = [];
  closeCalls = 0;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent<unknown>) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;

  constructor(readonly url: string) {
    FakeSocket.instances.push(this);
  }

  open() {
    this.readyState = 1;
    this.onopen?.(new Event("open"));
  }

  receive(message: ServerMsg) {
    this.onmessage?.({ data: JSON.stringify(message) } as MessageEvent<unknown>);
  }

  serverClose() {
    this.readyState = 3;
    this.onclose?.({ code: 1006 } as CloseEvent);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {
    this.closeCalls += 1;
    this.readyState = 3;
  }
}

function asWebSocket(socket: FakeSocket): WebSocket {
  return socket as unknown as WebSocket;
}

afterEach(() => {
  FakeSocket.instances = [];
  vi.useRealTimers();
});

describe("ChatSocket", () => {
  it("parse boundary and reconnect with Sync, backoff and ping", () => {
    vi.useFakeTimers();
    const sync: ServerMsg = { type: "sync", running: [], pending_confirms: [] };
    const onSync = vi.fn();
    const onMessage = vi.fn();
    const onStatus = vi.fn();
    const socket = new ChatSocket({
      socketFactory: (url) => asWebSocket(new FakeSocket(url)),
      onSync,
      onMessage,
      onStatus,
      random: () => 0,
    });

    socket.start();
    const first = FakeSocket.instances[0];
    expect(first.url).toMatch(/^ws:\/\/.*\/api\/ws$/);
    expect(socket.send({ type: "ping" })).toBe(false);
    first.open();
    first.receive(sync);
    expect(onSync).toHaveBeenNthCalledWith(1, sync, "initial");
    expect(socket.send({ type: "ping" })).toBe(true);
    expect(first.sent).toEqual([JSON.stringify({ type: "ping" })]);

    first.serverClose();
    expect(onStatus).toHaveBeenCalledWith("reconnecting");
    vi.advanceTimersByTime(499);
    expect(FakeSocket.instances).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(FakeSocket.instances).toHaveLength(2);
    const second = FakeSocket.instances[1];
    second.open();
    second.receive(sync);
    expect(onSync).toHaveBeenNthCalledWith(2, sync, "reconnect");
    vi.advanceTimersByTime(20_000);
    expect(second.sent).toEqual([JSON.stringify({ type: "ping" })]);
    socket.stop();
    expect(second.closeCalls).toBe(1);
  });

  it("bỏ JSON/message không hợp lệ", () => {
    expect(parseServerMessage({ type: "notification", session_id: 1, message_id: 2 })).toEqual({
      type: "notification",
      session_id: 1,
      message_id: 2,
    });
    expect(parseServerMessage({ type: "notification", session_id: 1 })).toBeNull();
    expect(parseServerMessage({ type: "script" })).toBeNull();
    expect(
      parseServerMessage({
        type: "text_delta",
        session_id: 1,
        run_id: "r",
        text: "x",
        index: 0,
        reset: true,
      }),
    ).not.toBeNull();
    expect(
      parseServerMessage({
        type: "text_delta",
        session_id: 1,
        run_id: "r",
        text: "x",
        index: -1,
        reset: true,
      }),
    ).toBeNull();
  });
});
