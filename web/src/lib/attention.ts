/**
 * Âm thanh chú ý: chime + đọc lời gọi tên khi Bean có việc cần người dùng chú ý.
 *
 * # Vì sao Web Audio API thay vì file mp3
 *
 * Một chime chỉ là hai nốt sine decay ~0,5s — sinh trực tiếp bằng oscillator thì
 * không cần tài nguyên nhị phân nào trong bundle (quy tắc không tải tài nguyên
 * ngoài), không phát sinh request, và đổi được cao độ theo ý muốn. Lời gọi tên
 * dùng `speechSynthesis` của trình duyệt — không gọi TTS server (voice M27 là
 * tính năng riêng, cần provider cấu hình; gọi tên phải chạy được cả khi voice
 * chưa dựng).
 *
 * # Vì sao lưu lựa chọn ở `localStorage` thay vì server
 *
 * Cùng lý do với thông báo nền (`lib/notify`): đây là tuỳ chọn của **thiết bị
 * này** (máy tính bật loa, điện thoại để im), không phải tuỳ chọn tài khoản.
 *
 * # Điều kiện phát
 *
 * Giống thông báo nền: chỉ phát khi cửa sổ Bean **không** hiển thị trước mắt
 * người dùng (`canPlayAttentionNow`). Đang nhìn màn hình chat thì thẻ xác nhận/
 * pill trạng thái đã là tín hiệu — âm thanh lúc đó chỉ là nhiễu. Ngoại lệ duy
 * nhất là nút "thử" (`testSoundAttention`): chạy trong thao tác bấm, vừa phản
 * hồi cho người dùng vừa gỡ khoá autoplay của trình duyệt cho những lần phát
 * sau khi tab ẩn (AudioContext/speechSynthesis cần một user gesture trước đó).
 */

import type { Lang, Translate } from "@/i18n";

/** Khoá lưu lựa chọn bật/tắt âm thanh trên thiết bị hiện tại. */
export const SOUND_STORAGE_KEY = "bean.sound";

/** Khoá lưu tên được gọi (ví dụ "Vinh Mr"); xoá hẳn khi để trống. */
export const SOUND_NAME_STORAGE_KEY = "bean.soundName";

/** Trần ký tự của tên gọi — chặn chuỗi dài bất thường dán nhầm vào ô nhập. */
export const NAME_MAX_LENGTH = 40;

/** Loại sự kiện chú ý — quyết định câu được đọc (theo ngôn ngữ UI). */
export type AttentionKind = "reply" | "confirm" | "push" | "error";

export function soundSupported(): boolean {
  return typeof window !== "undefined" && ("AudioContext" in window || "webkitAudioContext" in window);
}

export function speechSupported(): boolean {
  return typeof window !== "undefined" && "speechSynthesis" in window && "SpeechSynthesisUtterance" in window;
}

export function isSoundEnabled(): boolean {
  try {
    return window.localStorage.getItem(SOUND_STORAGE_KEY) === "1";
  } catch {
    // Chế độ riêng tư/storage bị chặn: coi như chưa bật, không báo lỗi ra UI.
    return false;
  }
}

export function enableSound(): void {
  try {
    window.localStorage.setItem(SOUND_STORAGE_KEY, "1");
  } catch {
    // Không lưu được thì lựa chọn chỉ sống trong phiên hiện tại — chấp nhận được.
  }
}

export function disableSound(): void {
  try {
    window.localStorage.removeItem(SOUND_STORAGE_KEY);
  } catch {
    // Bỏ qua — không còn gì để gỡ.
  }
}

/** Tên được gọi, đã trim và cắt ở ranh giới ký tự; rỗng nếu chưa đặt. */
export function getAttentionName(): string {
  try {
    const raw = window.localStorage.getItem(SOUND_NAME_STORAGE_KEY) ?? "";
    return Array.from(raw.trim()).slice(0, NAME_MAX_LENGTH).join("");
  } catch {
    return "";
  }
}

/** Lưu tên gọi; rỗng (hoặc toàn whitespace) thì xoá khoá — "chưa đặt" và "đã xoá" là một. */
export function setAttentionName(name: string): void {
  try {
    const clean = Array.from(name.trim()).slice(0, NAME_MAX_LENGTH).join("");
    if (clean === "") window.localStorage.removeItem(SOUND_NAME_STORAGE_KEY);
    else window.localStorage.setItem(SOUND_NAME_STORAGE_KEY, clean);
  } catch {
    // Không lưu được thì tên gọi chỉ sống trong phiên hiện tại — chấp nhận được.
  }
}

/**
 * Có được phép phát ngay bây giờ không: đã bật + thiết bị hỗ trợ + cửa sổ Bean
 * **không** đang hiển thị trước mắt người dùng (ẩn tab, cửa sổ khác chiếm focus).
 */
export function canPlayAttentionNow(): boolean {
  return (
    soundSupported() && isSoundEnabled() && (document.visibilityState !== "visible" || !document.hasFocus())
  );
}

/**
 * Câu được đọc cho một sự kiện. Tên rỗng thì dùng cách gọi chung của ngôn ngữ UI
 * ("Bạn ơi…" / "Hey there…") — câu đọc luôn có người gọi, không bao giờ bắt đầu
 * bằng dấu phẩy lơ lửng.
 */
export function attentionPhrase(kind: AttentionKind, name: string, t: Translate): string {
  const who = name.trim() === "" ? t("sound.defaultName") : name.trim();
  switch (kind) {
    case "reply":
      return t("sound.phrase.reply", { name: who });
    case "confirm":
      return t("sound.phrase.confirm", { name: who });
    case "push":
      return t("sound.phrase.push", { name: who });
    case "error":
      return t("sound.phrase.error", { name: who });
  }
}

/** Câu đọc cho nút "thử" — nói rõ âm thanh đã sẵn sàng thay vì chime trơ trọi. */
export function attentionTestPhrase(name: string, t: Translate): string {
  const who = name.trim() === "" ? t("sound.defaultName") : name.trim();
  return t("sound.testPhrase", { name: who });
}

// --- Phát chime bằng Web Audio API -------------------------------------------

let audioContext: AudioContext | null = null;

/**
 * Xoá `AudioContext` dùng chung — chỉ dành cho test: mỗi file test cài fake
 * riêng, cache giữa các bài test sẽ khiến chime đẩy nốt vào fake đã restore.
 * Ở chạy thật không ai gọi (singleton sống theo vòng đời trang là chủ đích).
 */
export function resetAudioContextForTests(): void {
  audioContext = null;
}

/**
 * Lấy `AudioContext` dùng chung (tạo lười — tạo sớm trong jsdom/trình duyệt cũ
 * là lỗi). Trình duyệt khởi tạo ở trạng thái `suspended` cho tới khi có user
 * gesture: gọi `resume()` mỗi lần phát, thất bại thì bỏ qua (không throw).
 */
function obtainAudioContext(): AudioContext | null {
  if (!soundSupported()) return null;
  if (audioContext === null) {
    const Ctor =
      window.AudioContext ??
      (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (Ctor === undefined) return null;
    audioContext = new Ctor();
  }
  if (audioContext.state === "suspended") void audioContext.resume().catch(() => undefined);
  return audioContext;
}

/** Biên độ đỉnh của chime — nhỏ, mục tiêu là "chú ý" chứ không phải "báo động". */
const CHIME_PEAK = 0.08;
/** Khoảng lặng giữa hai nốt (giây) — tạo nhịp "ding-dong" thay vì một tiếng ù. */
const CHIME_GAP = 0.16;
/** Thời gian decay mỗi nốt (giây). */
const CHIME_DECAY = 0.5;

function chimeTone(context: AudioContext, frequency: number, startAt: number): void {
  const oscillator = context.createOscillator();
  const gain = context.createGain();
  oscillator.type = "sine";
  oscillator.frequency.value = frequency;
  // `exponentialRampToValueAtTime` không chấp nhận 0 — dùng 0.0001 làm đáy im lặng.
  gain.gain.setValueAtTime(0.0001, startAt);
  gain.gain.exponentialRampToValueAtTime(CHIME_PEAK, startAt + 0.02);
  gain.gain.exponentialRampToValueAtTime(0.0001, startAt + CHIME_DECAY);
  oscillator.connect(gain);
  gain.connect(context.destination);
  oscillator.start(startAt);
  oscillator.stop(startAt + CHIME_DECAY + 0.05);
}

/** Hai nốt lệch nhau một quãng trưởng — đủ khác biệt để nhận ra là Bean gọi. */
function chime(): void {
  const context = obtainAudioContext();
  if (context === null) return;
  const now = context.currentTime;
  chimeTone(context, 880, now);
  chimeTone(context, 1108.7, now + CHIME_GAP);
}

// --- Đọc lời gọi tên bằng speechSynthesis ------------------------------------

/**
 * Chọn giọng đọc khớp ngôn ngữ UI (câu là tiếng Việt thì giọng phải là tiếng
 * Việt). `getVoices()` có thể trả rỗng trên Chrome cho tới khi `voiceschanged`
 * bắn — khi đó trả `null` và hệ điều hành tự chọn giọng mặc định.
 */
function pickVoice(preferredLang: Lang): SpeechSynthesisVoice | null {
  const voices = window.speechSynthesis.getVoices();
  return voices.find((voice) => voice.lang.toLowerCase().startsWith(preferredLang)) ?? null;
}

/** Đọc một câu; trả `true` nếu đã đưa vào hàng đọc. Câu mới đè câu cũ, không xếp hàng. */
export function speakAttention(phrase: string, preferredLang: Lang): boolean {
  if (!speechSupported()) return false;
  const text = phrase.trim();
  if (text === "") return false;
  try {
    const synth = window.speechSynthesis;
    // Sự kiện mới có ưu tiên cao hơn: nhiều sự kiện dồn lại thì đọc câu mới nhất.
    synth.cancel();
    const utterance = new SpeechSynthesisUtterance(text);
    const voice = pickVoice(preferredLang);
    if (voice !== null) utterance.voice = voice;
    utterance.rate = 1.05;
    synth.speak(utterance);
    return true;
  } catch {
    // Trình duyệt chặn (autoplay, chính sách trình bày): im lặng, người dùng
    // vẫn có thông báo nền.
    return false;
  }
}

// --- Điểm vào cho notifyEvents và nút "thử" ----------------------------------

/** Phát chime + đọc câu cho một sự kiện; trả `true` nếu có gì đó kêu ra loa. */
export function playAttention(kind: AttentionKind, name: string, t: Translate, lang: Lang): boolean {
  if (!canPlayAttentionNow()) return false;
  let played = false;
  if (soundSupported()) {
    chime();
    played = true;
  }
  if (speakAttention(attentionPhrase(kind, name, t), lang)) played = true;
  return played;
}

/**
 * Phát thử để kiểm tra cấu hình — bỏ qua điều kiện "tab đang ẩn" vì nút "thử"
 * nằm trong thao tác bấm (người dùng đang nhìn màn hình). Trả `true` nếu phát.
 */
export function testSoundAttention(name: string, t: Translate, lang: Lang): boolean {
  if (!isSoundEnabled() || !soundSupported()) return false;
  chime();
  return speakAttention(attentionTestPhrase(name, t), lang);
}
