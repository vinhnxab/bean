import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Translate } from "@/i18n";
import { vi as vietnamese } from "@/i18n/vi";
import {
  attentionPhrase,
  attentionTestPhrase,
  canPlayAttentionNow,
  disableSound,
  enableSound,
  getAttentionName,
  isSoundEnabled,
  NAME_MAX_LENGTH,
  playAttention,
  resetAudioContextForTests,
  SOUND_NAME_STORAGE_KEY,
  SOUND_STORAGE_KEY,
  setAttentionName,
  soundSupported,
  speakAttention,
  speechSupported,
  testSoundAttention,
} from "@/lib/attention";
import { installFakeAttention } from "@/test/attention";
import { stubPageContext } from "@/test/notify";

/**
 * Dùng từ điển thật thay vì `t` giả: câu đọc là copy người dùng nghe, test phải
 * khẳng định đúng CHUỖI ("Vinh Mr ơi, Bean trả lời xong rồi.") không phải chỉ
 * "đã tra từ điển".
 */
const t: Translate = (key, values) => {
  const template = vietnamese[key];
  if (!values) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in values ? String(values[name]) : match,
  );
};

describe("lib/attention — phát hiện API trong jsdom thật", () => {
  afterEach(() => {
    resetAudioContextForTests();
    window.localStorage.clear();
  });

  it("jsdom không có AudioContext/speechSynthesis thì mọi đường đều an toàn", () => {
    expect(soundSupported()).toBe(false);
    expect(speechSupported()).toBe(false);
    expect(canPlayAttentionNow()).toBe(false);
    expect(playAttention("reply", "", t, "vi")).toBe(false);
    expect(testSoundAttention("", t, "vi")).toBe(false);
    expect(speakAttention("xin chào", "vi")).toBe(false);
  });
});

describe("lib/attention — lựa chọn lưu localStorage", () => {
  let fake: ReturnType<typeof installFakeAttention>;

  beforeEach(() => {
    fake = installFakeAttention();
    window.localStorage.clear();
  });
  afterEach(() => {
    fake.restore();
    resetAudioContextForTests();
    window.localStorage.clear();
  });

  it("bật/tắt âm thanh ghi đúng khoá: bật là '1', tắt là xoá hẳn", () => {
    enableSound();
    expect(window.localStorage.getItem(SOUND_STORAGE_KEY)).toBe("1");
    expect(isSoundEnabled()).toBe(true);

    disableSound();
    expect(window.localStorage.getItem(SOUND_STORAGE_KEY)).toBeNull();
    expect(isSoundEnabled()).toBe(false);
  });

  it("lưu tên gọi: trim, cắt ở trần 40 ký tự đúng ranh giới ký tự", () => {
    setAttentionName("  Vinh Mr  ");
    expect(getAttentionName()).toBe("Vinh Mr");

    // 50 ký tự 'á' — cắt theo code point, không đứt dấu.
    setAttentionName("á".repeat(50));
    expect(Array.from(getAttentionName()).length).toBe(NAME_MAX_LENGTH);
    expect(window.localStorage.getItem(SOUND_NAME_STORAGE_KEY)).not.toContain(",");
  });

  it("tên rỗng/toàn whitespace thì xoá khoá — 'chưa đặt' và 'đã xoá' là một", () => {
    setAttentionName("Vinh Mr");
    setAttentionName("   ");
    expect(window.localStorage.getItem(SOUND_NAME_STORAGE_KEY)).toBeNull();
    expect(getAttentionName()).toBe("");
  });
});

describe("lib/attention — phát theo điều kiện tab ẩn", () => {
  let fake: ReturnType<typeof installFakeAttention>;

  beforeEach(() => {
    fake = installFakeAttention();
    window.localStorage.clear();
  });
  afterEach(() => {
    fake.restore();
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

  it("chưa bật: playAttention không chime, không đọc — dù tab đang ẩn", () => {
    withHiddenTab(() => {
      expect(playAttention("reply", "Vinh Mr", t, "vi")).toBe(false);
      expect(fake.tones).toHaveLength(0);
      expect(fake.spoken).toHaveLength(0);
    });
  });

  it("đã bật + tab ẩn: chime hai nốt + đọc câu với tên đã cấu hình", () => {
    enableSound();
    withHiddenTab(() => {
      expect(playAttention("reply", "Vinh Mr", t, "vi")).toBe(true);

      expect(fake.tones).toHaveLength(2);
      // Hai nốt khác cao độ, bắt đầu lệch nhau một chút — nhịp "ding-dong".
      const [firstTone, secondTone] = fake.tones;
      expect(firstTone?.frequency).not.toBe(secondTone?.frequency);
      expect(secondTone?.startAt ?? 0).toBeGreaterThan(firstTone?.startAt ?? 0);

      expect(fake.spoken).toHaveLength(1);
      expect(fake.spoken[0]?.text).toBe("Vinh Mr ơi, Bean trả lời xong rồi.");
    });
  });

  it("đang nhìn màn hình (visible + focused): không phát gì dù đã bật", () => {
    enableSound();
    // jsdom mặc định `hasFocus() = false` — phải stub đúng ngữ cảnh "đang nhìn".
    const restoreContext = stubPageContext({ visibility: "visible", focused: true });
    try {
      expect(playAttention("confirm", "Vinh Mr", t, "vi")).toBe(false);
      expect(fake.tones).toHaveLength(0);
      expect(fake.spoken).toHaveLength(0);
    } finally {
      restoreContext();
    }
  });

  it("tab ẩn nhưng visible + mất focus (cửa sổ khác đè): vẫn phát", () => {
    enableSound();
    const restoreContext = stubPageContext({ visibility: "visible", focused: false });
    try {
      expect(playAttention("push", "Vinh", t, "vi")).toBe(true);
      expect(fake.spoken[0]?.text).toBe("Vinh ơi, Bean vừa nhắn bạn.");
    } finally {
      restoreContext();
    }
  });

  it("chưa đặt tên: dùng cách gọi chung của ngôn ngữ UI, không bỏ trống chỗ tên", () => {
    enableSound();
    withHiddenTab(() => {
      playAttention("error", "   ", t, "vi");
      expect(fake.spoken[0]?.text).toBe("Bạn ơi, Bean gặp lỗi rồi.");
    });
  });

  it("mỗi loại sự kiện có câu riêng; testSoundAttention bỏ qua điều kiện tab ẩn", () => {
    enableSound();
    // Tab ĐANG NHÌN mà nút "thử" vẫn phải phát — đây là thao tác bấm của user.
    const restoreContext = stubPageContext({ visibility: "visible", focused: true });
    try {
      expect(testSoundAttention("Vinh Mr", t, "vi")).toBe(true);
      expect(fake.tones).toHaveLength(2);
      expect(fake.spoken[0]?.text).toBe("Vinh Mr ơi, âm thanh của Bean đã sẵn sàng.");

      expect(attentionPhrase("confirm", "Vinh", t)).toBe("Vinh ơi, Bean cần bạn duyệt một hành động.");
      expect(attentionTestPhrase("", t)).toBe("Bạn ơi, âm thanh của Bean đã sẵn sàng.");
    } finally {
      restoreContext();
    }
  });
});

describe("lib/attention — chọn giọng đọc", () => {
  let fake: ReturnType<typeof installFakeAttention>;

  beforeEach(() => {
    fake = installFakeAttention();
    window.localStorage.clear();
  });
  afterEach(() => {
    fake.restore();
    resetAudioContextForTests();
    window.localStorage.clear();
  });

  it("câu tiếng Việt thì nhặt giọng vi-VN, không lấy giọng đầu tiên trong danh sách", () => {
    expect(speakAttention("Vinh ơi", "vi")).toBe(true);
    expect(fake.spoken[0]?.voiceLang).toBe("vi-VN");
  });

  it("câu tiếng Anh thì nhặt giọng en-US", () => {
    expect(speakAttention("Hey Vinh", "en")).toBe(true);
    expect(fake.spoken[0]?.voiceLang).toBe("en-US");
  });

  it("không có giọng khớp: vẫn đọc bằng giọng mặc định (voice null) thay vì bỏ", () => {
    fake.setVoices([{ lang: "ja-JP" }]);
    expect(speakAttention("Vinh ơi", "vi")).toBe(true);
    expect(fake.spoken[0]?.voiceLang).toBeNull();
  });

  it("sự kiện mới đè sự kiện cũ: cancel được gọi trước mỗi lần speak", () => {
    speakAttention("câu một", "vi");
    speakAttention("câu hai", "vi");
    expect(fake.cancel).toHaveBeenCalledTimes(2);
  });
});
