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
