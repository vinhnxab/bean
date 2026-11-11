import { describe, expect, it } from "vitest";

import type { MessageDto } from "@/api/bindings";
import { isWorkMessage, mergeConfirmFlow, parseStoredMessage } from "@/features/chat/messages";
import type { ConfirmState } from "@/store/chat";

function dto(message: Record<string, unknown>): MessageDto {
  return {
    id: 1,
    session_id: 1,
    seq: 1,
    message: message as MessageDto["message"],
    created_at: "2026-09-28T00:00:00Z",
  };
}

const PNG_BASE64 = "iVBORw0KGgo=";

describe("parseStoredMessage — khối ảnh (M26)", () => {
  it("đọc khối ảnh hợp lệ", () => {
    const parsed = parseStoredMessage(
      dto({
        role: "tool",
        text: "Ảnh chụp màn hình",
        tool_call_id: "call-1",
        is_error: false,
        image: { media_type: "image/png", data: PNG_BASE64, sha256: "abc" },
      }),
    );
    expect(parsed?.image).toEqual({
      mediaType: "image/png",
      data: PNG_BASE64,
      sha256: "abc",
    });
  });

  it("message không có ảnh thì image là null", () => {
    const parsed = parseStoredMessage(dto({ role: "user", text: "chào" }));
    expect(parsed?.image).toBeNull();
  });

  it("bỏ khối ảnh có media_type ngoài tập cho phép", () => {
    // Không được để `image/svg+xml` lọt: giá trị này đi vào `data:` URI của thẻ
    // <img>, và lớp kiệm phải giống `ImageBlock::new` ở Rust.
    const parsed = parseStoredMessage(
      dto({
        role: "tool",
        text: "x",
        image: { media_type: "image/svg+xml", data: "<svg/>", sha256: "a" },
      }),
    );
    expect(parsed?.image).toBeNull();
  });

  it("bỏ khối ảnh thiếu trường hoặc sai kiểu", () => {
    for (const image of [
      { media_type: "image/png", data: PNG_BASE64 },
      { media_type: "image/png", data: 42, sha256: "a" },
      { data: PNG_BASE64, sha256: "a" },
      "không phải object",
      null,
    ]) {
      const parsed = parseStoredMessage(dto({ role: "tool", text: "x", image }));
      expect(parsed?.image).toBeNull();
    }
  });

  it("chấp nhận đủ ba MIME được phép", () => {
    for (const mediaType of ["image/png", "image/jpeg", "image/webp"]) {
      const parsed = parseStoredMessage(
        dto({
          role: "tool",
          text: "x",
          image: { media_type: mediaType, data: PNG_BASE64, sha256: "a" },
        }),
      );
      expect(parsed?.image?.mediaType).toBe(mediaType);
    }
  });
});

describe("isWorkMessage — ẩn quá trình làm việc của Bean trong dòng chat", () => {
  it("assistant kèm lời gọi tool là suy nghĩ trung gian → ẩn", () => {
    const parsed = parseStoredMessage(
      dto({
        role: "assistant",
        text: "để tôi xem file này",
        tool_calls: [{ id: "c1", name: "read_file", args: {} }],
      }),
    );
    expect(parsed && isWorkMessage(parsed)).toBe(true);
  });

  it("assistant không gọi tool là câu trả lời cuối → hiện", () => {
    const parsed = parseStoredMessage(dto({ role: "assistant", text: "xong rồi" }));
    expect(parsed && isWorkMessage(parsed)).toBe(false);
  });

  it("kết quả tool không kèm ảnh → ẩn; kèm ảnh → hiện (nội dung M26)", () => {
    const withImage = parseStoredMessage(
      dto({
        role: "tool",
        text: "ảnh chụp màn hình",
        tool_call_id: "c1",
        image: { media_type: "image/png", data: PNG_BASE64, sha256: "a" },
      }),
    );
    expect(withImage && isWorkMessage(withImage)).toBe(false);

    const noImage = parseStoredMessage(dto({ role: "tool", text: "output", tool_call_id: "c1" }));
    expect(noImage && isWorkMessage(noImage)).toBe(true);
  });

  it("tin của người dùng luôn hiện", () => {
    const parsed = parseStoredMessage(dto({ role: "user", text: "chào Bean" }));
    expect(parsed && isWorkMessage(parsed)).toBe(false);
  });
});

function confirmState(overrides: Partial<ConfirmState> = {}): ConfirmState {
  return {
    confirm_id: "c1",
    session_id: 1,
    run_id: "r1",
    prompt: "run_shell --command ls",
    risk: "confirm",
    allow_session_option: true,
    timeout_seconds: 300,
    role: null,
    receivedAt: Date.parse("2026-11-11T00:00:10Z"),
    resolution: "allowed",
    ...overrides,
  };
}

function msgDto(id: number, seq: number, role: string, text: string, createdAt: string): MessageDto {
  return {
    id,
    session_id: 1,
    seq,
    message: { role, text } as MessageDto["message"],
    created_at: createdAt,
  };
}

describe("mergeConfirmFlow — thẻ xác nhận nằm đúng dòng thời gian", () => {
  it("thẻ nằm sau tin kích hoạt và trước câu trả lời cuối tạo sau nó", () => {
    const flow = mergeConfirmFlow(
      [
        msgDto(1, 1, "user", "làm giúp", "2026-11-11T00:00:00Z"),
        msgDto(2, 2, "assistant", "xong rồi", "2026-11-11T00:00:20Z"),
      ],
      [confirmState()],
    );

    expect(flow.map((item) => item.kind)).toEqual(["message", "confirm", "message"]);
    expect(flow[0]?.kind === "message" && flow[0].message.id).toBe(1);
    expect(flow[2]?.kind === "message" && flow[2].message.id).toBe(2);
  });

  it("nhiều thẻ giữ thứ tự receivedAt bất kể thứ tự trong store", () => {
    const flow = mergeConfirmFlow(
      [
        msgDto(1, 1, "user", "làm giúp", "2026-11-11T00:00:00Z"),
        msgDto(2, 2, "assistant", "xong rồi", "2026-11-11T00:00:20Z"),
      ],
      [
        confirmState({ confirm_id: "c2", receivedAt: Date.parse("2026-11-11T00:00:15Z") }),
        confirmState({ confirm_id: "c1", receivedAt: Date.parse("2026-11-11T00:00:05Z") }),
      ],
    );

    expect(flow.map((item) => (item.kind === "confirm" ? item.confirm.confirm_id : "msg"))).toEqual([
      "msg",
      "c1",
      "c2",
      "msg",
    ]);
  });

  it("thẻ đang chờ (mới hơn mọi tin) nằm đáy — đúng hành vi khi run đang chạy", () => {
    const flow = mergeConfirmFlow(
      [msgDto(1, 1, "user", "làm giúp", "2026-11-11T00:00:00Z")],
      [confirmState({ resolution: "pending", receivedAt: Date.now() })],
    );

    expect(flow.map((item) => item.kind)).toEqual(["message", "confirm"]);
  });

  it("thẻ của run trước nằm trước tin của run sau", () => {
    const flow = mergeConfirmFlow(
      [
        msgDto(1, 1, "user", "run một", "2026-11-11T00:00:00Z"),
        msgDto(2, 2, "assistant", "xong run một", "2026-11-11T00:00:20Z"),
        msgDto(3, 3, "user", "run hai", "2026-11-11T01:00:00Z"),
      ],
      [confirmState()],
    );

    expect(flow.map((item) => item.kind)).toEqual(["message", "confirm", "message", "message"]);
    expect(flow[3]?.kind === "message" && flow[3].message.id).toBe(3);
  });

  it("created_at không parse được thì thẻ xếp sau tin đó", () => {
    const flow = mergeConfirmFlow(
      [msgDto(1, 1, "user", "làm giúp", "không-phải-thời-gian")],
      [confirmState()],
    );

    expect(flow.map((item) => item.kind)).toEqual(["message", "confirm"]);
  });

  it("không có thẻ thì luồng giữ nguyên danh sách tin", () => {
    const messages = [
      msgDto(1, 1, "user", "chào", "2026-11-11T00:00:00Z"),
      msgDto(2, 2, "assistant", "hi", "2026-11-11T00:00:01Z"),
    ];
    const flow = mergeConfirmFlow(messages, []);

    expect(flow).toHaveLength(2);
    expect(flow.every((item) => item.kind === "message")).toBe(true);
  });
});
