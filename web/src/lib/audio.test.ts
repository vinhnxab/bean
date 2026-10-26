import { describe, expect, it } from "vitest";

import { encodeWav } from "./audio";

describe("encodeWav", () => {
  it("ghi header WAV PCM 16-bit mono với sample rate cho trước", async () => {
    const blob = encodeWav([new Float32Array([0, 0.5, -0.5])], 16_000);
    const view = new DataView(await blob.arrayBuffer());
    const ascii = (offset: number, length: number) =>
      String.fromCharCode(...new Uint8Array(view.buffer, offset, length));

    expect(ascii(0, 4)).toBe("RIFF");
    expect(ascii(8, 4)).toBe("WAVE");
    expect(ascii(12, 4)).toBe("fmt ");
    expect(ascii(36, 4)).toBe("data");
    expect(view.getUint16(20, true)).toBe(1); // PCM
    expect(view.getUint16(22, true)).toBe(1); // mono
    expect(view.getUint32(24, true)).toBe(16_000);
    expect(view.getUint16(34, true)).toBe(16); // bits per sample
    expect(view.getUint32(4, true)).toBe(36 + 3 * 2); // RIFF size
    expect(view.getUint32(40, true)).toBe(3 * 2); // data size
    expect(blob.type).toBe("audio/wav");
    expect(blob.size).toBe(44 + 3 * 2);
  });

  it("chuyển sample Float32 -1..1 sang Int16 và kẹp ngoài phạm vi", async () => {
    const blob = encodeWav([new Float32Array([0, 1, -1, 2, -3])], 16_000);
    const view = new DataView(await blob.arrayBuffer());
    const sample = (index: number) => view.getInt16(44 + index * 2, true);

    expect(sample(0)).toBe(0);
    expect(sample(1)).toBe(32_767); // 1 → 0x7fff
    expect(sample(2)).toBe(-32_768); // -1 → 0x8000
    expect(sample(3)).toBe(32_767); // kẹp từ 2
    expect(sample(4)).toBe(-32_768); // kẹp từ -3
  });

  it("nối nhiều chunk liên tiếp, đúng tổng số sample", async () => {
    const blob = encodeWav([new Float32Array([0.25]), new Float32Array([-0.25, 0.75])], 22_050);
    const view = new DataView(await blob.arrayBuffer());
    const sample = (index: number) => view.getInt16(44 + index * 2, true);

    expect(view.getUint32(24, true)).toBe(22_050);
    expect(view.getUint32(40, true)).toBe(3 * 2);
    // setInt16 truncate phần thập phân; số âm nhân 0x8000 (phạm vi âm có 32768 giá trị),
    // số dương nhân 0x7fff — khớp cách encodeWav map [-1,1] sang Int16.
    expect(sample(0)).toBe(Math.trunc(0.25 * 0x7fff));
    expect(sample(1)).toBe(Math.trunc(-0.25 * 0x8000));
    expect(sample(2)).toBe(Math.trunc(0.75 * 0x7fff));
  });
});
