import { useQuery } from "@tanstack/react-query";
import { waitFor } from "@testing-library/react";
import { HttpResponse, http } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { api } from "@/api/client";
import { queryKeys } from "@/features/auth/queries";
import { RealtimeProvider } from "@/features/chat/RealtimeProvider";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

/**
 * Test RealtimeProvider ở tầng giả lập socket thật: đẩy JSON vào `onmessage` của
 * WebSocket giả, khẳng định hiệu ứng ở phía **REST** (số lần gọi handler) — cùng
 * cách lật ngược trừu tượng như server test dùng `tokio-tungstenite`.
 *
 * `notification` (M13) là sự kiện dễ quên nhất vì nó không thuộc một run: nếu
 * provider quên invalidate, tin chủ động của scheduler chỉ xuất hiện sau khi
 * người dùng tự tải lại trang.
 */

/** WebSocket giả mà test giữ được tham chiếu cuối để đẩy sự kiện server vào. */
let lastSocket: FakeWebSocket | null = null;

class FakeWebSocket {
  readyState = 1;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;

  constructor(_url: string) {
    lastSocket = this;
  }

  send(): void {}
  close(): void {
    this.onclose?.();
  }

  receive(message: unknown): void {
    this.onmessage?.({ data: JSON.stringify(message) as MessageEvent["data"] } as MessageEvent);
  }
}

/** Handler msw đếm lượt gọi — lượt gọi tăng là bằng chứng query được làm mới. */
function countingHandler(url: string, body: Parameters<typeof HttpResponse.json>[0]) {
  let calls = 0;
  return {
    handler: http.get(url, () => {
      calls += 1;
      return HttpResponse.json(body);
    }),
    calls: () => calls,
  };
}

/** Quan sát đúng ba query mà provider phải làm mới khi nhận `notification`. */
function Probe() {
  useQuery({ queryKey: queryKeys.sessions, queryFn: ({ signal }) => api.listSessions({}, signal) });
  useQuery({
    queryKey: queryKeys.messages(7),
    queryFn: ({ signal }) => api.listMessages(7, null, signal),
  });
  useQuery({ queryKey: queryKeys.skillDrafts, queryFn: ({ signal }) => api.listSkillDrafts(signal) });
  return null;
}

/** Quan sát query `/api/status` — banner restart (M31) phụ thuộc nó để tắt. */
function StatusProbe() {
  useQuery({ queryKey: queryKeys.status, queryFn: ({ signal }) => api.status(signal) });
  return null;
}

const STATUS_BODY = {
  version: "test",
  model: "fake-model",
  max_steps: 25,
  daily_token_budget: 1_000_000,
  tokens_used: 0,
  uptime_seconds: 1,
  channels: ["web"],
  restart: { required: false, reasons: [] },
};

describe("RealtimeProvider — notification (M13)", () => {
  beforeEach(() => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    lastSocket = null;
  });

  it("làm mới messages của session, sessions và skill drafts", async () => {
    const sessions = countingHandler("/api/sessions", { sessions: [] });
    const messages = countingHandler("/api/sessions/7/messages", { messages: [] });
    const drafts = countingHandler("/api/skills/drafts", { drafts: [] });
    testServer.use(sessions.handler, messages.handler, drafts.handler);

    renderManagement(
      <RealtimeProvider>
        <Probe />
      </RealtimeProvider>,
    );
    await waitFor(() => expect(lastSocket).not.toBeNull());
    lastSocket?.onopen?.();

    // Chờ lần fetch đầu xong để so sánh được số lượt trước/sau sự kiện.
    await waitFor(() => {
      expect(sessions.calls()).toBeGreaterThan(0);
      expect(messages.calls()).toBeGreaterThan(0);
      expect(drafts.calls()).toBeGreaterThan(0);
    });
    const before = { s: sessions.calls(), m: messages.calls(), d: drafts.calls() };

    lastSocket?.receive({ type: "notification", session_id: 7, message_id: 42 });

    await waitFor(() => {
      expect(messages.calls()).toBeGreaterThan(before.m);
      expect(sessions.calls()).toBeGreaterThan(before.s);
      expect(drafts.calls()).toBeGreaterThan(before.d);
    });
  });

  it("sự kiện quá trình (text_delta) không chạm REST", async () => {
    const sessions = countingHandler("/api/sessions", { sessions: [] });
    const messages = countingHandler("/api/sessions/7/messages", { messages: [] });
    const drafts = countingHandler("/api/skills/drafts", { drafts: [] });
    testServer.use(sessions.handler, messages.handler, drafts.handler);

    renderManagement(
      <RealtimeProvider>
        <Probe />
      </RealtimeProvider>,
    );
    await waitFor(() => expect(lastSocket).not.toBeNull());
    lastSocket?.onopen?.();
    await waitFor(() => expect(messages.calls()).toBeGreaterThan(0));
    const before = { s: sessions.calls(), m: messages.calls(), d: drafts.calls() };

    lastSocket?.receive({
      type: "text_delta",
      session_id: 7,
      run_id: "run-7",
      index: 0,
      reset: true,
      text: "chào",
    });

    // Cho React một nhịp để mọi invalidate (nếu có do lỗi) kịp phát sinh.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(sessions.calls()).toBe(before.s);
    expect(messages.calls()).toBe(before.m);
    expect(drafts.calls()).toBe(before.d);
  });
});

describe("RealtimeProvider — banner restart realtime (M31)", () => {
  beforeEach(() => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    lastSocket = null;
  });

  it("WS nối lại xong bấm refetch /api/status NGAY, không chờ chu kỳ 30 giây", async () => {
    // Backoff nối lại (500ms + jitter) vẫn chạy theo real time nhờ
    // `shouldAdvanceTime` — MSW và `waitFor` không bị treo bởi fake timers.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const status = countingHandler("/api/status", STATUS_BODY);
      testServer.use(status.handler);

      renderManagement(
        <RealtimeProvider>
          <StatusProbe />
        </RealtimeProvider>,
      );
      await waitFor(() => expect(lastSocket).not.toBeNull());
      lastSocket?.onopen?.();
      await waitFor(() => expect(status.calls()).toBeGreaterThan(0));
      const first = lastSocket;
      const before = status.calls();

      // Đứt kết nối (tiến trình cũ tắt êm) → ChatSocket xếp lại lịch nối.
      first?.close();
      // Nối lại đi qua timer backoff — chờ socket MỚI được tạo; đẩy sự kiện
      // lên socket cũ thì `awaitingSyncAfterReconnect` chưa set, reason sẽ
      // không phải "reconnect" và kịch bản trở nên vô nghĩa.
      await waitFor(() => expect(lastSocket).not.toBe(first));
      lastSocket?.onopen?.();
      lastSocket?.receive({ type: "sync", running: [], pending_confirms: [] });

      // Sync trên socket mới ⇒ reason "reconnect" ⇒ invalidate `status` ⇒
      // refetch ngay trong nhịp đó — tín hiệu để banner restart tắt realtime.
      await waitFor(() => expect(status.calls()).toBeGreaterThan(before));
    } finally {
      vi.useRealTimers();
    }
  });
});
