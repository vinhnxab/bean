import type { SessionDto } from "@/api/bindings";
import type { MessageKey } from "@/i18n/vi";

/**
 * Gom hội thoại thành các nhóm theo mốc thời gian, cùng cách Open WebUI chia
 * danh sách chat (Hôm nay / Hôm qua / 7 ngày / 30 ngày / cũ hơn).
 *
 * Vì sao phải có: danh sách phẳng khiến người dùng phải đọc ngày tháng ở từng
 * dòng để biết cái nào cũ. Nhóm theo mốc quen thuộc trả lời được câu hỏi
 * "hôm nay tôi làm gì" mà không cần nhìn ngày.
 *
 * Mốc tính theo **thời gian địa phương của trình duyệt** (`startOfDay`), không
 * phải UTC: người dùng nghĩ "hôm nay" theo đồng hồ của họ. Server trả RFC3339
 * UTC, nên phải chuyển sang `Date` (tự parse offset) rồi mới so sánh.
 */
export type SessionGroupKey = "today" | "yesterday" | "week" | "month" | "older" | "archived";

export type SessionGroup = {
  key: SessionGroupKey;
  /** Danh sách hội thoại, đã sắp xếp mới nhất trước. */
  sessions: SessionDto[];
};

const GROUP_LABEL: Record<SessionGroupKey, MessageKey> = {
  today: "sessions.group.today",
  yesterday: "sessions.group.yesterday",
  week: "sessions.group.week",
  month: "sessions.group.month",
  older: "sessions.group.older",
  archived: "sessions.group.archived",
};

/** Thứ tự hiển thị cố định — nhóm mới hơn luôn ở trên. */
const GROUP_ORDER: SessionGroupKey[] = ["today", "yesterday", "week", "month", "older"];

const DAY_MS = 86_400_000;

function startOfDay(value: Date): number {
  return new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
}

/**
 * Nhóm của một hội thoại. Session **đã lưu trữ** luôn vào nhóm riêng bất kể tuổi
 * — vì người dùng bật chế độ "xem đã lưu" là để dọn dẹp, không phải để đọc lại
 * theo tuổi.
 */
export function groupOfSession(session: SessionDto, now: Date): SessionGroupKey {
  if (session.archived) return "archived";
  const updated = new Date(session.updated_at || session.created_at);
  // Ngày không đọc được (chuỗi rỗng/hỏng) → coi như cũ, để nó nằm cuối danh
  // sách thay vì biến mất. Không ném lỗi: đây là dữ liệu từ server, lỗi hiển
  // thị phải không được làm sập cả trang.
  if (Number.isNaN(updated.getTime())) return "older";
  const days = Math.floor((startOfDay(now) - startOfDay(updated)) / DAY_MS);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 7) return "week";
  if (days < 30) return "month";
  return "older";
}

/** Khoá dịch cho tên nhóm. */
export function groupLabelKey(key: SessionGroupKey): MessageKey {
  return GROUP_LABEL[key];
}

/**
 * Gom và sắp xếp danh sách hội thoại. Trả về mảng rỗng nếu không có gì — đơn
 * vị hóa đầu ra để component không phải tự xử lý `null`.
 */
export function groupSessions(sessions: SessionDto[], now: Date): SessionGroup[] {
  const buckets = new Map<SessionGroupKey, SessionDto[]>();
  for (const session of sessions) {
    const key = groupOfSession(session, now);
    const bucket = buckets.get(key);
    if (bucket) bucket.push(session);
    else buckets.set(key, [session]);
  }
  const result: SessionGroup[] = [];
  for (const key of GROUP_ORDER) {
    const bucket = buckets.get(key);
    if (bucket) result.push({ key, sessions: sortByUpdated(bucket) });
  }
  const archived = buckets.get("archived");
  if (archived) result.push({ key: "archived", sessions: sortByUpdated(archived) });
  return result;
}

/** Mới nhất trước; hội thoại chưa có `updated_at` rơi xuống cuối. */
function sortByUpdated(sessions: SessionDto[]): SessionDto[] {
  return [...sessions].sort((left, right) => {
    const a = new Date(left.updated_at || left.created_at).getTime();
    const b = new Date(right.updated_at || right.created_at).getTime();
    if (Number.isNaN(a) && Number.isNaN(b)) return 0;
    if (Number.isNaN(a)) return 1;
    if (Number.isNaN(b)) return -1;
    return b - a;
  });
}
