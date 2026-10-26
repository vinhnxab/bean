/**
 * Chuyển đổi audio cho voice push-to-talk (M27).
 *
 * # Vì sao base64 thay vì multipart
 *
 * REST của Bean bắt buộc `Content-Type: application/json` cho mọi request thay đổi
 * dữ liệu (chống CSRF). Clip push-to-talk chỉ dài vài giây nên chi phí base64 (~33%)
 * chấp nhận được, đổi lại không phải nới lỏng bất biến bảo mật của server.
 */

/**
 * Giải mã base64 thành `ArrayBuffer`. `atob` chỉ cho chuỗi Latin-1 nên phải đọc từng mã.
 *
 * Trả `ArrayBuffer` (không phải `Uint8Array`) để dùng thẳng làm `BlobPart`: view
 * `Uint8Array` mang kiểu `ArrayBufferLike` (có thể là `SharedArrayBuffer`) nên TS
 * không nhận, còn `ArrayBuffer` thì luôn hợp lệ.
 */
export function base64ToArrayBuffer(base64: string): ArrayBuffer {
  const binary = atob(base64);
  const buffer = new ArrayBuffer(binary.length);
  const view = new Uint8Array(buffer);
  for (let index = 0; index < binary.length; index += 1) {
    view[index] = binary.charCodeAt(index);
  }
  return buffer;
}

/** Mã hoá `Blob` thành base64 (không kèm tiền tố `data:`) để gửi trong JSON. */
export async function blobToBase64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/**
 * Sample rate mong muốn cho ghi âm STT: `whisper.cpp` đọc WAV 16 kHz mono tốt nhất.
 * `AudioContext` với `sampleRate` này khiến trình duyệt tự resample luồng micro,
 * khỏi phải tự viết bộ resample phía JS.
 */
const RECORD_SAMPLE_RATE = 16_000;

/** Kích thước buffer mỗi lần callback của `ScriptProcessorNode` (số sample). */
const RECORD_BUFFER_SIZE = 4_096;

function writeAscii(view: DataView, offset: number, text: string): void {
  for (let index = 0; index < text.length; index += 1) {
    view.setUint8(offset + index, text.charCodeAt(index));
  }
}

/**
 * Đóng gói các đoạn PCM Float32 (giá trị -1..1) thành `Blob` WAV 16-bit mono.
 *
 * # Vì sao WAV thay vì `MediaRecorder` mặc định
 *
 * `MediaRecorder` không chỉ định `mimeType` sẽ ghi `audio/webm;codecs=opus` trên
 * Chrome — bản `whisper-cli` thường thấy (không build kèm FFmpeg) chỉ giải mã
 * được WAV PCM, nên STT luôn thất bại. Ghi PCM trực tiếp qua WebAudio và tự đóng
 * header WAV giúp server nhận đúng định dạng mà không cần binary phụ (ffmpeg).
 */
export function encodeWav(chunks: readonly Float32Array[], sampleRate: number): Blob {
  const totalSamples = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const buffer = new ArrayBuffer(44 + totalSamples * 2);
  const view = new DataView(buffer);

  writeAscii(view, 0, "RIFF");
  view.setUint32(4, 36 + totalSamples * 2, true);
  writeAscii(view, 8, "WAVE");
  writeAscii(view, 12, "fmt ");
  view.setUint32(16, 16, true); // kích thước chunk fmt cho PCM
  view.setUint16(20, 1, true); // format PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true); // byte rate = rate * 2 byte/sample
  view.setUint16(32, 2, true); // block align
  view.setUint16(34, 16, true); // bits per sample
  writeAscii(view, 36, "data");
  view.setUint32(40, totalSamples * 2, true);

  let offset = 44;
  for (const chunk of chunks) {
    for (let index = 0; index < chunk.length; index += 1) {
      // Kẹp về -1..1 rồi nén về Int16; hai nửa phạm vi khác nhau vì Int16 không đối xứng.
      const sample = Math.max(-1, Math.min(1, chunk[index]));
      view.setInt16(offset, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
      offset += 2;
    }
  }
  return new Blob([buffer], { type: "audio/wav" });
}

/**
 * Trình ghi âm micro xuất WAV 16-bit mono cho STT (thay `MediaRecorder`).
 *
 * # Bất biến
 *
 * * `stop()`/`cancel()` luôn nhả micro và đóng `AudioContext` — gọi được đúng một lần,
 *   an toàn cả khi AudioContext chưa kịp khởi tạo xong (webapp ghi giữa chừng).
 * * `onaudioprocess` nhận buffer tái sử dụng của trình duyệt — phải `slice()` sao chép
 *   ngay trong callback, không giữ tham chiếu.
 */
export class WavRecorder {
  private chunks: Float32Array[] = [];
  private stopped = false;

  private constructor(
    private readonly context: AudioContext,
    private readonly stream: MediaStream,
    private readonly node: ScriptProcessorNode,
    private readonly source: MediaStreamAudioSourceNode,
  ) {}

  /** Xin quyền micro và bắt đầu thu PCM. Ném lỗi khi người dùng từ chối/thiết bị lỗi. */
  static async start(): Promise<WavRecorder> {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    const context = new AudioContext({ sampleRate: RECORD_SAMPLE_RATE });
    const source = context.createMediaStreamSource(stream);
    const node = context.createScriptProcessor(RECORD_BUFFER_SIZE, 1, 1);
    const recorder = new WavRecorder(context, stream, node, source);
    node.onaudioprocess = (event) => {
      if (!recorder.stopped) {
        recorder.chunks.push(event.inputBuffer.getChannelData(0).slice());
      }
    };
    // ScriptProcessorNode chỉ chạy khi có kết nối tới destination; output mặc định
    // im lặng nên không gây hồi âm micro ra loa.
    source.connect(node);
    node.connect(context.destination);
    return recorder;
  }

  /** Dừng ghi, nhả micro và trả WAV sẵn sàng gửi STT. */
  stop(): Blob {
    this.cleanup();
    return encodeWav(this.chunks, this.context.sampleRate);
  }

  /** Dừng ghi và bỏ dữ liệu — dùng khi unmount giữa chừng. */
  cancel(): void {
    this.cleanup();
    this.chunks = [];
  }

  private cleanup(): void {
    this.stopped = true;
    this.node.onaudioprocess = null;
    this.node.disconnect();
    this.source.disconnect();
    for (const track of this.stream.getTracks()) track.stop();
    void this.context.close().catch(() => undefined);
  }
}

/**
 * Phát audio base64 và hoàn tất khi phát xong (hoặc khi lỗi).
 *
 * Dùng object URL của `Blob` thay vì `data:` URL: CSP của Bean chỉ cho phép
 * `media-src 'self' blob:` — dữ liệu `data:` bị chặn, còn `blob:` là tài nguyên cùng
 * nguồn gốc. Thu hồi URL ngay sau khi phát để không rò bộ nhớ.
 */
export async function playBase64Audio(base64: string, mime: string): Promise<void> {
  const blob = new Blob([base64ToArrayBuffer(base64)], { type: mime });
  const url = URL.createObjectURL(blob);
  const audio = new Audio(url);
  try {
    await audio.play();
    await new Promise<void>((resolve) => {
      audio.onended = () => resolve();
      audio.onerror = () => resolve();
      // Trình duyệt chặn autoplay: coi như xong, người dùng vẫn đọc được tin nhắn.
      audio.onpause = () => resolve();
    });
  } finally {
    URL.revokeObjectURL(url);
  }
}
