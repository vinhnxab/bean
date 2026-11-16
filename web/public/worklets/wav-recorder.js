/**
 * Bộ ghi âm PCM cho voice push-to-talk (K32) — chạy trong AudioWorkletGlobalScope.
 *
 * # Vì sao là file tĩnh trong public/ chứ không bundle
 *
 * CSP của Bean là `default-src 'self'` (không khai `worker-src`/`blob:`), nên
 * `audioWorklet.addModule` phải trỏ tới một file cùng nguồn gốc. File trong
 * `public/` được Vite giữ nguyên và phục vụ tại `/worklets/wav-recorder.js`.
 * Đây là JS thuần, không import — AudioWorkletGlobalScope không có module system
 * của trang.
 *
 * # Giao thức với host (web/src/lib/audio.ts)
 *
 * * `processorOptions.flushSamples` — số sample tối đa của một gói dữ liệu.
 * * Gom sample đầu vào vào buffer nội bộ; khi đủ `flushSamples` thì gửi một
 *   **bản sao** `Float32Array` qua `port` rồi làm rỗng buffer. Phải copy vì host
 *   tái sử dụng bộ đệm giữa các lần render (bất biến cũ của ScriptProcessorNode).
 * * Không gửi phần đuôi chưa đầy gói khi dừng — host đóng context, phần lẻ bị
 *   bỏ, đúng hành vi cũ (mất đuôi ~256ms là parity, không phải lỗi).
 */
class WavRecorderProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const requested = options?.processorOptions?.flushSamples;
    this.buffer = new Float32Array(
      Number.isFinite(requested) && requested > 0 ? requested : 4096,
    );
    this.filled = 0;
  }

  process(inputs) {
    const input = inputs[0];
    const channel = input && input.length > 0 ? input[0] : null;
    if (channel) {
      const capacity = this.buffer.length;
      for (let index = 0; index < channel.length; index += 1) {
        this.buffer[this.filled] = channel[index];
        this.filled += 1;
        if (this.filled === capacity) {
          this.port.postMessage(this.buffer.slice(0));
          this.filled = 0;
        }
      }
    }
    // Trả true để giữ processor sống; output mặc định im lặng (không hồi âm).
    return true;
  }
}

registerProcessor("wav-recorder", WavRecorderProcessor);
