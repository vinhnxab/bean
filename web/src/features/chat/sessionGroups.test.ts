import { describe, expect, it } from "vitest";

import type { SessionDto } from "@/api/bindings";
import { groupOfSession, groupSessions } from "@/features/chat/sessionGroups";

/**
 * Đồng hồ cố định để test không phụ thuộc ngày thật khi chạy.
 *
 * Và sao dựng bằng `new Date(2026, 8, 29, 12)` chứ không dùng chuỗi ISO:
 * chuỗi `"2026-09-28T20:00:00Z"` là UTC, còn hàm so sánh dùng `startOfDay` theo
 * giờ **địa phương**. Ở UTC+7 nó rơi sang 29/09 giờ địa phương nên thành "hôm nay",
 * ở UTC lại là "hôm qua" — cùng một test fail ở một nửa thế giới. Dựng `Date`
 * từ thành phần ngày/giờ địa phương khiến ý nghĩa của test là "19 giờ trước,
 * theo đồng hồ máy", đúng như cách người dùng thật nhìn danh sách.
 */
const NOW = new Date(2026, 8, 29, 12, 0, 0);

/** Ngày cụ thể theo giờ địa phương, chuyển sang chuỗi như server trả về. */
function localIso(year: number, month: number, day: number, hour = 10, minute = 0): string {
  return new Date(year, month, day, hour, minute).toISOString();
}

function session(id: number, iso: string, archived = false): SessionDto {
  return {
    id,
    channel: "web",
    chat_id: "web:admin",
    user_id: "web:admin",
    title: `Phiên ${id}`,
    archived,
    created_at: iso,
    updated_at: iso,
  };
}

describe("groupOfSession", () => {
  it("xếp đúng các mốc ngày quanh mốc hiện tại", () => {
    // 29/9 01:00 — sáng nay, sau `NOW` (12:00) vẫn là hôm nay.
    expect(groupOfSession(session(1, localIso(2026, 8, 29, 1)), NOW)).toBe("today");
    // 28/9 23:00 — tối qua dù mới hơn `NOW` chỉ 13 giờ.
    expect(groupOfSession(session(2, localIso(2026, 8, 28, 23)), NOW)).toBe("yesterday");
    expect(groupOfSession(session(3, localIso(2026, 8, 27, 10)), NOW)).toBe("week");
    expect(groupOfSession(session(4, localIso(2026, 8, 15, 10)), NOW)).toBe("month");
    expect(groupOfSession(session(5, localIso(2026, 5, 1, 10)), NOW)).toBe("older");
  });

  it("rạn đúng ở ranh giới 7 ngày và 30 ngày", () => {
    // 22/9 là 7 ngày trước 29/9 → rơi sang "30 ngày trước", không phải "7 ngày".
    expect(groupOfSession(session(10, localIso(2026, 8, 22, 12)), NOW)).toBe("month");
    // 23/9 mới đúng 6 ngày.
    expect(groupOfSession(session(11, localIso(2026, 8, 23, 12)), NOW)).toBe("week");
    // 30/8 là 30 ngày trước → "Tháng trước".
    expect(groupOfSession(session(12, localIso(2026, 7, 30, 12)), NOW)).toBe("older");
    expect(groupOfSession(session(13, localIso(2026, 7, 31, 12)), NOW)).toBe("month");
  });

  it("session đã lưu trữ luôn vào nhóm riêng bất kể tuổi", () => {
    expect(groupOfSession(session(6, localIso(2026, 8, 29, 1), true), NOW)).toBe("archived");
    expect(groupOfSession(session(7, localIso(2020, 0, 1), true), NOW)).toBe("archived");
  });

  it("ngày hỏng không làm hỏng danh sách, chỉ đẩy xuống nhóm cũ", () => {
    expect(groupOfSession(session(8, ""), NOW)).toBe("older");
    expect(groupOfSession(session(9, "không-phải-ngày"), NOW)).toBe("older");
  });
});

describe("groupSessions", () => {
  it("trả về mảng rỗng khi không có hội thoại nào", () => {
    expect(groupSessions([], NOW)).toEqual([]);
  });

  it("giữ thứ tự nhóm cố định và sắp mới nhất trước trong mỗi nhóm", () => {
    const groups = groupSessions(
      [
        session(1, localIso(2026, 8, 10, 10)), // 19 ngày trước -> month
        session(2, localIso(2026, 8, 29, 1)), // today (cũ hơn)
        session(3, localIso(2026, 8, 29, 9)), // today (mới hơn)
        session(4, localIso(2026, 8, 28, 10)), // yesterday
      ],
      NOW,
    );
    expect(groups.map((group) => group.key)).toEqual(["today", "yesterday", "month"]);
    expect(groups[0].sessions.map((item) => item.id)).toEqual([3, 2]);
  });

  it("nhóm archived luôn nằm cuối, không lẫn vào nhóm theo tuổi", () => {
    const groups = groupSessions(
      [session(1, localIso(2026, 5, 1), true), session(2, localIso(2026, 8, 29, 9))],
      NOW,
    );
    expect(groups.map((group) => group.key)).toEqual(["today", "archived"]);
  });
});
