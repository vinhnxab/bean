import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ServerMsg } from "@/api/bindings";
import { maybeAlertServerMessage, notifyContentFor } from "@/features/chat/notifyEvents";
import type { Translate } from "@/i18n";
import { vi as vietnamese } from "@/i18n/vi";
import { enableSound, resetAudioContextForTests } from "@/lib/attention";
import { NOTIFY_STORAGE_KEY } from "@/lib/notify";
import { installFakeAttention } from "@/test/attention";
import { type FakeNotificationInstance, installFakeNotifications, stubPageContext } from "@/test/notify";

/**
 * Dùng từ điển `vi` thật thay vì `t` giả: nội dung thông báo là copy người dùng
 * đọc, test phải khẳng định đúng CHUỖI hiển thị ("Bean đã trả lời xong"), không
 * phải chỉ "đã tra từ điển".
 */
const t: Translate = (key, values) => {
  const template = vietnamese[key];
  if (!values) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in values ? String(values[name]) : match,
  );
};

describe("features/chat/notifyEvents — nội dung thông báo", () => {
  it("final: tiêu đề trả lời xong + preview câu trả lời, tag theo run", () => {
    const message: Extract<ServerMsg, { type: "final" }> = {
      type: "final",
      session_id: 9,
      run_id: "run-1",
      message_id: 55,
      text: "Đã tạo ghi-chu.md với 3 việc cần làm hôm nay.",
    };

    const content = notifyContentFor(message, t);

    expect(content).toEqual({
      title: "Bean đã trả lời xong",
      body: "Đã tạo ghi-chu.md với 3 việc cần làm hôm nay.",
      tag: "bean:final:run-1",
      sessionId: 9,
    });
  });

  it("final với text rỗng: dùng câu thay thế thay vì body trống trơn", () => {
    const message: Extract<ServerMsg, { type: "final" }> = {
      type: "final",
      session_id: 9,
      run_id: "run-1",
      message_id: 55,
      text: "   \n  ",
    };

    const content = notifyContentFor(message, t);

    expect(content?.body).toBe("Có câu trả lời mới trong hội thoại.");
  });

  it("preview: gộp whitespace, cắt 120 ký tự đúng ranh giới ký tự, có dấu ellipsis", () => {
    const long = `X${"ó".repeat(200)} ${"🦊".repeat(30)}`;
    const message: Extract<ServerMsg, { type: "final" }> = {
      type: "final",
      session_id: 1,
      run_id: "r",
      message_id: 1,
      text: `dòng 1\n\ndòng   2 — ${long}`,
    };

    const content = notifyContentFor(message, t);

    expect(content?.body.startsWith("dòng 1 dòng 2 — X")).toBe(true);
    expect(content?.body.endsWith("…")).toBe(true);
    // Trần 120 ký tự + dấu "…" — đếm theo code point, không đứt surrogate pair.
    expect(Array.from(content?.body ?? "").length).toBe(121);
  });

  it("confirm_request: tiêu đề cần xác nhận + nguyên văn hành động (preview)", () => {
    const message: Extract<ServerMsg, { type: "confirm_request" }> = {
      type: "confirm_request",
      session_id: 3,
      run_id: "run-2",
      confirm_id: "c-abc",
      prompt: "Viết file ghi-chu.md (9 dòng)",
      risk: "confirm",
      allow_session_option: true,
      timeout_seconds: 300,
      role: null,
    };

    const content = notifyContentFor(message, t);

    expect(content?.title).toBe("Cần xác nhận hành động");
    expect(content?.body).toBe("Viết file ghi-chu.md (9 dòng)");
    expect(content?.tag).toBe("bean:confirm:c-abc");
    expect(content?.sessionId).toBe(3);
  });

  it("notification (scheduler/learning): không có payload chữ — dùng câu mô tả", () => {
    const message: Extract<ServerMsg, { type: "notification" }> = {
      type: "notification",
      session_id: 12,
      message_id: 77,
    };

    const content = notifyContentFor(message, t);

    expect(content?.title).toBe("Bean gửi tin nhắn mới");
    expect(content?.body).toContain("tác vụ định kỳ");
    expect(content?.tag).toBe("bean:notification:12:77");
  });

  it("error toàn cục (session null): sessionId null + tag global — nhấp chỉ focus", () => {
    const message: Extract<ServerMsg, { type: "error" }> = {
      type: "error",
      session_id: null,
      run_id: null,
      code: "budget",
      message: "Ngân sách token/ngày đã hết.",
    };

    const content = notifyContentFor(message, t);

    expect(content?.title).toBe("Bean gặp lỗi khi xử lý");
    expect(content?.sessionId).toBeNull();
    expect(content?.tag).toBe("bean:error:global:none");
  });

  it("sự kiện quá trình (delta, tool) KHÔNG có nội dung thông báo", () => {
    const delta: Extract<ServerMsg, { type: "text_delta" }> = {
      type: "text_delta",
      session_id: 1,
      run_id: "r",
      text: "một chút",
      index: 0,
      reset: false,
    };
    const toolStart: Extract<ServerMsg, { type: "tool_start" }> = {
      type: "tool_start",
      session_id: 1,
      run_id: "r",
      id: "t1",
      tool: "read_file",
      summary: "Đọc file",
      args_preview: "{}",
    };

    expect(notifyContentFor(delta, t)).toBeNull();
    expect(notifyContentFor(toolStart, t)).toBeNull();
  });
});

describe("features/chat/notifyEvents — bắn và chống trùng", () => {
  let fake: ReturnType<typeof installFakeNotifications>;
  let attention: ReturnType<typeof installFakeAttention>;

  const finalMsg = (runId: string): Extract<ServerMsg, { type: "final" }> => ({
    type: "final",
    session_id: 9,
    run_id: runId,
    message_id: 1,
    text: "xong",
  });

  beforeEach(() => {
    fake = installFakeNotifications();
    attention = installFakeAttention();
    fake.setPermission("granted");
    window.localStorage.setItem(NOTIFY_STORAGE_KEY, "1");
  });
  afterEach(() => {
    fake.restore();
    attention.restore();
    resetAudioContextForTests();
    window.localStorage.clear();
  });

  function withHiddenTab(run: () => void): void {
    const restoreContext = stubPageContext({ visibility: "hidden", focused: false });
    try {
      run();
    } finally {
      restoreContext();
    }
  }

  it("đủ điều kiện thì bắn đúng một lần; sự kiện lặp lại không bắn thêm", () => {
    withHiddenTab(() => {
      const seen: string[] = [];

      expect(maybeAlertServerMessage(finalMsg("run-1"), t, "vi", seen)).toBe(true);
      // Socket phát lại cùng sự kiện (lag/replay): khoá đã ghi — không bắn lần 2.
      expect(maybeAlertServerMessage(finalMsg("run-1"), t, "vi", seen)).toBe(false);
      expect(fake.instances).toHaveLength(1);
    });
  });

  it("tab đang hiển thị: không bắn NHƯNG vẫn ghi khoá (sự kiện đã đi qua)", () => {
    // jsdom mặc định `hasFocus() = false` — phải stub đúng ngữ cảnh "đang nhìn".
    const restoreContext = stubPageContext({ visibility: "visible", focused: true });
    try {
      const seen: string[] = [];

      expect(maybeAlertServerMessage(finalMsg("run-1"), t, "vi", seen)).toBe(false);
      expect(fake.instances).toHaveLength(0);
      expect(attention.spoken).toHaveLength(0);
      expect(seen).toContain("final:run-1");
    } finally {
      restoreContext();
    }
  });

  it("text_delta không được ghi khoá lẫn bắn (eventKey null)", () => {
    const seen: string[] = [];
    const delta: Extract<ServerMsg, { type: "text_delta" }> = {
      type: "text_delta",
      session_id: 1,
      run_id: "r",
      text: "a",
      index: 0,
      reset: false,
    };

    expect(maybeAlertServerMessage(delta, t, "vi", seen)).toBe(false);
    expect(seen).toHaveLength(0);
  });

  it("bộ nhớ khoá có trần 200: khoá cũ bị bỏ để phiên lâu không phình vô hạn", () => {
    withHiddenTab(() => {
      const seen: string[] = [];
      for (let index = 0; index < 205; index += 1) {
        maybeAlertServerMessage(finalMsg(`run-${index}`), t, "vi", seen);
      }
      expect(seen).toHaveLength(200);

      // Khoá cổ nhất ("final:run-0") đã bị đẩy ra — bắn lại là hợp lệ.
      expect(maybeAlertServerMessage(finalMsg("run-0"), t, "vi", seen)).toBe(true);
    });
  });

  it("bắn thật thì instance mang đúng tiêu đề tiếng Việt từ từ điển", () => {
    withHiddenTab(() => {
      const seen: string[] = [];
      maybeAlertServerMessage(finalMsg("run-1"), t, "vi", seen);

      const instance = fake.instances[0] as FakeNotificationInstance;
      expect(instance.title).toBe("Bean đã trả lời xong");
      expect(instance.body).toBe("xong");
    });
  });

  it("âm thanh: đã bật + tab ẩn thì chime + đọc câu gọi tên; chưa bật thì im", () => {
    withHiddenTab(() => {
      const seen: string[] = [];
      // Âm thanh chưa bật: chỉ thông báo nền.
      expect(maybeAlertServerMessage(finalMsg("run-1"), t, "vi", seen)).toBe(true);
      expect(attention.spoken).toHaveLength(0);

      enableSound();
      // Khoá "final:run-1" đã ghi ở trên — sự kiện cũ không kêu lại; run mới thì có.
      expect(maybeAlertServerMessage(finalMsg("run-2"), t, "vi", seen)).toBe(true);
      expect(attention.spoken).toHaveLength(1);
      expect(attention.spoken[0]?.text).toBe("Bạn ơi, Bean trả lời xong rồi.");
      expect(attention.tones).toHaveLength(2);
    });
  });

  it("âm thanh theo loại sự kiện: confirm/notification/error có câu riêng", () => {
    withHiddenTab(() => {
      enableSound();
      const seen: string[] = [];

      const confirmMsg: Extract<ServerMsg, { type: "confirm_request" }> = {
        type: "confirm_request",
        session_id: 3,
        run_id: "run-2",
        confirm_id: "c-1",
        prompt: "Viết file ghi-chu.md",
        risk: "confirm",
        allow_session_option: true,
        timeout_seconds: 300,
        role: null,
      };
      const pushMsg: Extract<ServerMsg, { type: "notification" }> = {
        type: "notification",
        session_id: 12,
        message_id: 77,
      };
      const errorMsg: Extract<ServerMsg, { type: "error" }> = {
        type: "error",
        session_id: 1,
        run_id: "run-3",
        code: "budget",
        message: "Ngân sách token/ngày đã hết.",
      };

      maybeAlertServerMessage(confirmMsg, t, "vi", seen);
      maybeAlertServerMessage(pushMsg, t, "vi", seen);
      maybeAlertServerMessage(errorMsg, t, "vi", seen);

      expect(attention.spoken.map((item) => item.text)).toEqual([
        "Bạn ơi, Bean cần bạn duyệt một hành động.",
        "Bạn ơi, Bean vừa nhắn bạn.",
        "Bạn ơi, Bean gặp lỗi rồi.",
      ]);
    });
  });

  it("một sự kiện bắn cả hai kênh khi cả hai đều bật", () => {
    withHiddenTab(() => {
      enableSound();
      const seen: string[] = [];

      expect(maybeAlertServerMessage(finalMsg("run-1"), t, "vi", seen)).toBe(true);
      expect(fake.instances).toHaveLength(1);
      expect(attention.spoken).toHaveLength(1);
    });
  });
});
