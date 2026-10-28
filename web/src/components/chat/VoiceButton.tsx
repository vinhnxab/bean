import { MicIcon, SquareIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { ApiRequestError, api } from "@/api/client";
import { useI18n } from "@/i18n";
import { blobToBase64, WavRecorder } from "@/lib/audio";
import { cn } from "@/lib/utils";

/** Trạng thái nút mic: rảnh → ghi âm → đang nhận dạng → rảnh. */
type Phase = "idle" | "recording" | "transcribing";

/**
 * Trần thời lượng một lần ghi (giây): WAV 16 kHz nặng ~32 KB/giây nên ghi quên
 * tắt sẽ phình payload vượt trần body của server — tự dừng thay vì để STT hỏng 413.
 */
const MAX_RECORD_SECONDS = 60;

export type VoiceButtonProps = {
  /** Vô hiệu hoá khi socket chưa kết nối (không gửi được lượt nào). */
  disabled: boolean;
  /** Transcript sau khi nhận dạng; chỉ gọi khi có nội dung. */
  onResult: (text: string) => void;
};

/**
 * Nút mic push-to-talk (M27).
 *
 * # Vì sao bấm để bật/tắt thay vì giữ để nói
 *
 * Giữ chuột/phím trên thiết bị di động và với bàn phím rất dễ "tuột" PTT giữa câu,
 * và không có sự kiện bàn phím nào mô tả đúng "nhả" khi trợ năng đang bật. Bấm-bật/
 * bấm-tắt cho kết quả y hệt với thao tác rõ ràng, hoạt động cả bằng Enter/Space.
 *
 * # Bất biến
 *
 * * Micro luôn được nhả trong `finally`/cleanup — không để icon micro sáng mãi.
 * * Không gọi `onResult` với chuỗi rỗng (im lặng hoặc nhận dạng hụt).
 * * Lỗi STT hiện dưới ô soạn, không ném ra làm vỡ giao diện.
 */
export function VoiceButton({ disabled, onResult }: VoiceButtonProps) {
  const { t } = useI18n();
  const [phase, setPhase] = useState<Phase>("idle");
  const [error, setError] = useState<string | null>(null);
  const recorderRef = useRef<WavRecorder | null>(null);

  const releaseRecorder = useCallback(() => {
    recorderRef.current?.cancel();
    recorderRef.current = null;
  }, []);

  // Rời màn hình giữa lúc ghi: nhả micro, bỏ dữ liệu. `cancel()` idempotent nên
  // an toàn cả khi đã `stop()` trước đó.
  useEffect(() => releaseRecorder, [releaseRecorder]);

  const transcribe = useCallback(
    async (blob: Blob) => {
      // WAV chỉ có header 44 byte = không capture được sample nào (context bị
      // suspend, thiết bị im lặng tuyệt đối…) — báo lỗi riêng thay vì hỏi STT.
      if (blob.size <= 44) {
        setPhase("idle");
        setError(t("chat.voice.noAudio"));
        return;
      }
      try {
        const audio_base64 = await blobToBase64(blob);
        const result = await api.transcribe({ audio_base64, mime: "audio/wav" });
        setPhase("idle");
        const text = result.text.trim();
        if (text) onResult(text);
      } catch (error) {
        // Chi tiết từ server (timeout, binary/model lỗi, mã thoát…) giúp tự chẩn
        // đoán thay vì một câu chung chung; console giữ nguyên lỗi đầy đủ.
        console.error("voice transcribe failed", error);
        setPhase("idle");
        const detail = error instanceof ApiRequestError && error.message ? error.message : null;
        setError(detail ? `${t("chat.voice.error")} — ${detail}` : t("chat.voice.error"));
      }
    },
    [onResult, t],
  );

  const start = useCallback(async () => {
    setError(null);
    if (!navigator.mediaDevices?.getUserMedia || typeof AudioContext === "undefined") {
      setError(t("chat.voice.unsupported"));
      return;
    }
    try {
      // Ghi WAV PCM 16 kHz: `whisper-cli` (không build kèm FFmpeg) chỉ giải mã
      // được WAV — `MediaRecorder` mặc định ghi webm/opus làm STT luôn thất bại.
      recorderRef.current = await WavRecorder.start();
      setPhase("recording");
    } catch {
      // Người dùng từ chối quyền, hoặc không có thiết bị ghi âm.
      releaseRecorder();
      setError(t("chat.voice.denied"));
    }
  }, [releaseRecorder, t]);

  const stop = useCallback(() => {
    const recorder = recorderRef.current;
    if (recorder) {
      recorderRef.current = null;
      setPhase("transcribing");
      void transcribe(recorder.stop());
    }
  }, [transcribe]);

  const toggle = useCallback(() => {
    if (phase === "recording") {
      stop();
    } else if (phase === "idle") {
      void start();
    }
  }, [phase, start, stop]);

  // Ghi quá trần thời lượng: tự dừng (timer chỉ sống khi đang ghi, hủy khi stop).
  useEffect(() => {
    if (phase !== "recording") return;
    const timer = setTimeout(stop, MAX_RECORD_SECONDS * 1000);
    return () => clearTimeout(timer);
  }, [phase, stop]);

  const recording = phase === "recording";
  const label = recording
    ? t("chat.voice.stop")
    : phase === "transcribing"
      ? t("chat.voice.transcribing")
      : t("chat.voice.start");

  return (
    <div className="flex shrink-0 flex-col items-center gap-1">
      <button
        type="button"
        onClick={toggle}
        disabled={disabled || phase === "transcribing"}
        aria-label={label}
        aria-pressed={recording}
        title={label}
        className={cn(
          "mb-1 flex size-8 items-center justify-center rounded-lg border transition-colors",
          recording
            ? "border-live text-live hover:bg-live hover:text-live-ink"
            : "border-rule text-muted-foreground hover:border-brand hover:text-brand",
          "disabled:pointer-events-none disabled:opacity-40",
        )}
      >
        {recording ? <SquareIcon className="size-3.5 fill-current" /> : <MicIcon className="size-4" />}
      </button>
      {error ? (
        <p className="max-w-[14rem] text-center text-xs text-alert" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
