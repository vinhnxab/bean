import { describe, expect, it } from "vitest";

import { base64urlToUint8Array } from "@/lib/push";

/**
 * Test phần thuần của `lib/push` — decode base64url dùng cho
 * `applicationServerKey`. Phần còn lại của module cần service worker/PushManager
 * thật (jsdom không có), không test được ở đây; hành vi đăng ký được vận hành
 * qua test backend (`tests/push_rest.rs`) và test SW (`service-worker.test.ts`).
 */
describe("lib/push base64urlToUint8Array", () => {
  it("decode đúng các vector RFC 4648 §10 dạng base64url", () => {
    expect([...base64urlToUint8Array("")]).toEqual([]);
    expect([...base64urlToUint8Array("Zg")]).toEqual([0x66]);
    expect([...base64urlToUint8Array("Zm8")]).toEqual([0x66, 0x6f]);
    expect([...base64urlToUint8Array("Zm9v")]).toEqual([0x66, 0x6f, 0x6f]);
  });

  it("decode đúng chuỗi có ký tự - và _ (no-pad)", () => {
    // 0x00 0xff 0x01 → base64 "AP8B" — không cần padding vì bội số của 3 byte.
    expect([...base64urlToUint8Array("AP8B")]).toEqual([0x00, 0xff, 0x01]);
    // 0xfb 0xff 0xbe → base64 "++/+" chứa cả '+' và '/' — đổi sang base64url
    // bằng biến đổi chuẩn để tránh lỗi đánh máy literal.
    const urlSafe = btoa(String.fromCharCode(0xfb, 0xff, 0xbe))
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/g, "");
    expect([...base64urlToUint8Array(urlSafe)]).toEqual([251, 255, 190]);
    // 251..256 byte tuần tự: kiểm cả khối lớn (public key VAPID là 65 byte).
    const bytes = Array.from({ length: 256 }, (_, i) => i);
    const encoded = btoa(String.fromCharCode(...bytes))
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/g, "");
    expect([...base64urlToUint8Array(encoded)]).toEqual(bytes);
  });

  it("chấp nhận cả chuỗi còn padding chuẩn", () => {
    expect([...base64urlToUint8Array("Zm9vYmFy")]).toEqual([0x66, 0x6f, 0x6f, 0x62, 0x61, 0x72]);
    expect([...base64urlToUint8Array("Zg==")]).toEqual([0x66]);
  });
});
