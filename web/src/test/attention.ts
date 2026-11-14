import { vi } from "vitest";

/**
 * Fake `AudioContext` + `speechSynthesis` cho jsdom (jsdom KHÔNG có cả hai).
 *
 * Dùng chung cho test của `lib/attention` và mọi nơi gọi phát âm thanh: thay
 * bằng lớp giả giữ đúng hình dáng API thật (oscillator ghi lại nốt được bắt
 * đầu, speechSynthesis ghi lại câu được đọc cùng giọng đã chọn) để code sản
 * phẩm chạy thật trên đó thay vì bị mock lọt qua everywhere.
 *
 * # Quy ước khôi phục
 *
 * Mỗi file test gọi `installFakeAttention()` trong `beforeEach` và `restore()`
 * trong `afterEach` — không dùng `vi.stubGlobal` vì descriptor gốc của jsdom
 * phải giữ nguyên chính xác cho các test khác (jsdom thật là "không hỗ trợ").
 */

export type FakeTone = {
  frequency: number;
  startAt: number;
};

export type FakeSpoken = {
  text: string;
  voiceLang: string | null;
};

export function installFakeAttention() {
  const tones: FakeTone[] = [];
  const spoken: FakeSpoken[] = [];
  // Hai giọng: tiếng Anh trước, tiếng Việt sau — test khẳng định bộ chọn giọng
  // nhặt đúng giọng khớp ngôn ngữ thay vì lấy giọng đầu tiên.
  const voices: { lang: string }[] = [{ lang: "en-US" }, { lang: "vi-VN" }];

  class FakeOscillator {
    type = "sine";
    frequency = { value: 0 };
    connect = vi.fn().mockReturnThis();
    start = vi.fn((startAt?: number) => {
      tones.push({ frequency: this.frequency.value, startAt: startAt ?? 0 });
    });
    stop = vi.fn();
    disconnect = vi.fn();
  }

  class FakeGain {
    gain = {
      value: 1,
      setValueAtTime: vi.fn(),
      exponentialRampToValueAtTime: vi.fn(),
    };
    connect = vi.fn().mockReturnThis();
    disconnect = vi.fn();
  }

  class FakeAudioContext {
    state = "running";
    currentTime = 1;
    destination = {};
    resume = vi.fn(async () => {});
    createOscillator = vi.fn(() => new FakeOscillator());
    createGain = vi.fn(() => new FakeGain());
    close = vi.fn(async () => {});
  }

  class FakeUtterance {
    text: string;
    lang = "";
    voice: { lang: string } | null = null;
    rate = 1;
    volume = 1;
    constructor(text: string) {
      this.text = text;
    }
  }

  const synth = {
    cancel: vi.fn(),
    speak: vi.fn((utterance: FakeUtterance) => {
      spoken.push({
        text: utterance.text,
        voiceLang: utterance.voice === null ? null : utterance.voice.lang,
      });
    }),
    getVoices: vi.fn(() => voices),
  };

  const audioDescriptor = Object.getOwnPropertyDescriptor(window, "AudioContext");
  const synthDescriptor = Object.getOwnPropertyDescriptor(window, "speechSynthesis");
  const utteranceDescriptor = Object.getOwnPropertyDescriptor(window, "SpeechSynthesisUtterance");
  Object.defineProperty(window, "AudioContext", {
    configurable: true,
    writable: true,
    value: FakeAudioContext,
  });
  Object.defineProperty(window, "speechSynthesis", { configurable: true, writable: true, value: synth });
  Object.defineProperty(window, "SpeechSynthesisUtterance", {
    configurable: true,
    writable: true,
    value: FakeUtterance,
  });

  return {
    tones,
    spoken,
    cancel: synth.cancel,
    setVoices: (next: { lang: string }[]) => {
      voices.splice(0, voices.length, ...next);
    },
    restore: () => {
      for (const [name, descriptor] of [
        ["AudioContext", audioDescriptor],
        ["speechSynthesis", synthDescriptor],
        ["SpeechSynthesisUtterance", utteranceDescriptor],
      ] as const) {
        if (descriptor) Object.defineProperty(window, name, descriptor);
        else delete (window as unknown as Record<string, unknown>)[name];
      }
    },
  };
}
