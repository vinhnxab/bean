import { describe, expect, it } from "vitest";

import type { MessageDto } from "@/api/bindings";
import { parseStoredMessage } from "@/features/chat/messages";

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
