import { describe, expect, it } from "vitest";

import type { ServerMsg } from "@/api/bindings";
import { parseServerMessage } from "@/api/ws";
import { useChatStore } from "@/store/chat";

const sync: Extract<ServerMsg, { type: "sync" }> = { type: "sync", running: [], pending_confirms: [] };

describe("chat event state", () => {
  it("xử lý chuỗi Queued → ToolStart → ToolEnd → Final và refetch Final", () => {
    useChatStore.getState().reset();
    const queued = useChatStore
      .getState()
      .applyServerMessage({ type: "queued", session_id: 1, run_id: "run-1", position: 1 });
    const started = useChatStore.getState().applyServerMessage({
      type: "tool_start",
      session_id: 1,
      run_id: "run-1",
      id: "call-1",
      tool: "read_file",
      summary: "read_file",
      args_preview: "path=a.txt",
    });
    const ended = useChatStore.getState().applyServerMessage({
      type: "tool_end",
      session_id: 1,
      run_id: "run-1",
      id: "call-1",
      ok: true,
      output_preview: "ok",
    });
    const final = useChatStore.getState().applyServerMessage({
      type: "final",
      session_id: 1,
      run_id: "run-1",
      message_id: 99,
      text: "xong",
    });

    expect(queued).toBeNull();
    expect(started?.sessionId).toBe(1);
    expect(ended?.sessionId).toBe(1);
    expect(final).toEqual({ sessionId: 1, messageId: 99, runId: "run-1" });
    expect(useChatStore.getState().runsBySession[1]?.status).toBe("idle");
    expect(
      useChatStore.getState().applyServerMessage({
        type: "final",
        session_id: 1,
        run_id: "run-1",
        message_id: 99,
        text: "xong",
      }),
    ).toBeNull();
  });

  it("ghép delta, giữ text khi reconnect và refetch Final", () => {
    useChatStore.getState().reset();
    useChatStore.getState().applyServerMessage({
      type: "text_delta",
      session_id: 1,
      run_id: "run-1",
      text: "",
      index: 0,
      reset: true,
    });
    useChatStore.getState().applyServerMessage({
      type: "text_delta",
      session_id: 1,
      run_id: "run-1",
      text: "Xin",
      index: 1,
      reset: false,
    });
    useChatStore
      .getState()
      .applySync(
        { type: "sync", running: [{ session_id: 1, run_id: "run-1", role: null }], pending_confirms: [] },
        true,
      );
    expect(useChatStore.getState().runsBySession[1]?.streamText).toBe("Xin");

    useChatStore.getState().applyServerMessage({
      type: "text_delta",
      session_id: 1,
      run_id: "run-1",
      text: " chào",
      index: 2,
      reset: false,
    });
    expect(useChatStore.getState().runsBySession[1]?.streamText).toBe("Xin chào");

    useChatStore.getState().applyServerMessage({
      type: "text_delta",
      session_id: 1,
      run_id: "run-0",
      text: "delta cũ",
      index: 0,
      reset: true,
    });
    expect(useChatStore.getState().runsBySession[1]?.streamText).toBe("Xin chào");

    useChatStore.getState().applyServerMessage({
      type: "text_delta",
      session_id: 1,
      run_id: "run-1",
      text: "Lần sau",
      index: 0,
      reset: true,
    });
    expect(useChatStore.getState().runsBySession[1]?.streamText).toBe("Lần sau");

    const final = useChatStore.getState().applyServerMessage({
      type: "final",
      session_id: 1,
      run_id: "run-1",
      message_id: 42,
      text: "xong",
    });
    expect(final).toEqual({ sessionId: 1, messageId: 42, runId: "run-1" });
    expect(useChatStore.getState().runsBySession[1]?.streamText).toBe("");
  });

  it("bỏ event trùng theo run_id và chấp nhận Notification không có run_id", () => {
    useChatStore.getState().reset();
    expect(parseServerMessage({ type: "notification", session_id: 1, message_id: 3 })).not.toBeNull();
    expect(parseServerMessage(sync)).not.toBeNull();
    const first = useChatStore
      .getState()
      .applyServerMessage({ type: "text", session_id: 1, run_id: "run-2", text: "x" });
    const second = useChatStore
      .getState()
      .applyServerMessage({ type: "text", session_id: 1, run_id: "run-2", text: "x" });
    expect(first?.runId).toBe("run-2");
    expect(second).toBeNull();
  });

  it("error event (session_id number) đưa run về idle kèm thông báo lỗi", () => {
    useChatStore.getState().reset();
    useChatStore.getState().markSubmitting(7);
    useChatStore.getState().applyServerMessage({
      type: "error",
      session_id: 7,
      run_id: "run-7",
      code: "llm",
      message: "provider trả HTTP 401: User not found.",
    });
    const run = useChatStore.getState().runsBySession[7];
    expect(run?.status).toBe("idle");
    expect(run?.error).toBe("provider trả HTTP 401: User not found.");
    expect(run?.errorCode).toBe("llm");
    expect(run?.runId).toBeNull();
  });

  it("applySync reconcile reset run submitting (runId null) thành interrupted", () => {
    useChatStore.getState().reset();
    useChatStore.getState().markSubmitting(3);
    useChatStore.getState().applySync(sync, true);
    const run = useChatStore.getState().runsBySession[3];
    expect(run?.status).toBe("idle");
    expect(run?.errorCode).toBe("interrupted");
    expect(run?.error).toBeNull();
  });

  it("applySync giữ run submitting khi lần sync đầu (không reconcile)", () => {
    useChatStore.getState().reset();
    useChatStore.getState().markSubmitting(3);
    useChatStore.getState().applySync(sync, false);
    expect(useChatStore.getState().runsBySession[3]?.status).toBe("submitting");
  });

  it("applySync ghi đè interrupted bằng run thật server đang chạy", () => {
    useChatStore.getState().reset();
    useChatStore.getState().markSubmitting(3);
    useChatStore
      .getState()
      .applySync(
        { type: "sync", running: [{ session_id: 3, run_id: "run-3", role: null }], pending_confirms: [] },
        true,
      );
    const run = useChatStore.getState().runsBySession[3];
    expect(run?.status).toBe("running");
    expect(run?.runId).toBe("run-3");
    expect(run?.errorCode).toBeNull();
  });
});
