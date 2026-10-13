import { MicIcon, SquareIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "@/api/client";
import { useI18n } from "@/i18n";
import { blobToBase64 } from "@/lib/audio";
import { cn } from "@/lib/utils";

/** Trạng thái nút mic: rảnh → ghi âm → đang nhận dạng → rảnh. */
type Phase = "idle" | "recording" | "transcribing";

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
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);

  const releaseStream = useCallback(() => {
    for (const track of streamRef.current?.getTracks() ?? []) track.stop();
    streamRef.current = null;
  }, []);

  // Rời màn hình giữa lúc ghi: dừng recorder và nhả micro. Gỡ `onstop` trước khi
  // `stop()` để không gọi `setState`/`onResult` trên component đã unmount.
  useEffect(
    () => () => {
      const recorder = recorderRef.current;
      if (recorder) {
        recorder.onstop = null;
        if (recorder.state !== "inactive") recorder.stop();
      }
      releaseStream();
    },
    [releaseStream],
  );

  const transcribe = useCallback(
    async (mime: string) => {
      const blob = new Blob(chunksRef.current, { type: mime || "audio/webm" });
      chunksRef.current = [];
      if (blob.size === 0) {
        setPhase("idle");
        return;
      }
      try {
        const audio_base64 = await blobToBase64(blob);
        const result = await api.transcribe({
          audio_base64,
          mime: blob.type || "audio/webm",
        });
        setPhase("idle");
        const text = result.text.trim();
        if (text) onResult(text);
      } catch {
        setPhase("idle");
        setError(t("chat.voice.error"));
      }
    },
    [onResult, t],
  );

  const start = useCallback(async () => {
    setError(null);
    if (typeof MediaRecorder === "undefined" || !navigator.mediaDevices?.getUserMedia) {
      setError(t("chat.voice.unsupported"));
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      streamRef.current = stream;
      chunksRef.current = [];
      const recorder = new MediaRecorder(stream);
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) chunksRef.current.push(event.data);
      };
      recorder.onstop = () => {
        void transcribe(recorder.mimeType);
      };
      recorderRef.current = recorder;
      recorder.start();
      setPhase("recording");
    } catch {
      // Người dùng từ chối quyền, hoặc không có thiết bị ghi âm.
      releaseStream();
      setError(t("chat.voice.denied"));
    }
  }, [releaseStream, t, transcribe]);

  const stop = useCallback(() => {
    const recorder = recorderRef.current;
    if (recorder && recorder.state !== "inactive") {
      setPhase("transcribing");
      recorder.stop();
    }
    releaseStream();
  }, [releaseStream]);

  const toggle = useCallback(() => {
    if (phase === "recording") {
      stop();
    } else if (phase === "idle") {
      void start();
    }
  }, [phase, start, stop]);

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
          "flex size-8 items-center justify-center rounded-lg border transition-colors",
          recording
            ? "border-live text-live hover:bg-live hover:text-live-ink"
            : "border-rule text-muted-foreground hover:border-brand hover:text-brand",
          "disabled:pointer-events-none disabled:opacity-40",
        )}
      >
        {recording ? <SquareIcon className="size-3.5 fill-current" /> : <MicIcon className="size-4" />}
      </button>
      {error ? (
        <p className="max-w-[9rem] text-center text-xs text-alert" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
