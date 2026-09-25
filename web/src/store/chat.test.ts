import { describe, expect, it } from "vitest";

import { parseServerMessage } from "@/api/ws";
import { useChatStore } from "@/store/chat";

const sync = { type: "sync", running: [], pending_confirms: [] } as const;

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
    const final = useChatStore
      .getState()
      .applyServerMessage({ type: "final", session_id: 1, run_id: "run-1", message_id: 99 });

    expect(queued).toBeNull();
    expect(started?.sessionId).toBe(1);
    expect(ended?.sessionId).toBe(1);
    expect(final).toEqual({ sessionId: 1, messageId: 99, runId: "run-1" });
    expect(useChatStore.getState().runsBySession[1]?.status).toBe("idle");
    expect(
      useChatStore
        .getState()
        .applyServerMessage({ type: "final", session_id: 1, run_id: "run-1", message_id: 99 }),
    ).toBeNull();
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
});
