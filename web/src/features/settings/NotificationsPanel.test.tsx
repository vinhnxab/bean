import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { NotificationsPanel } from "@/features/settings/NotificationsPanel";
import { resetAudioContextForTests } from "@/lib/attention";
import { installFakeAttention } from "@/test/attention";
import { renderManagement } from "@/test/management";
import { installFakeNotifications } from "@/test/notify";

/**
 * Panel chỉ phụ thuộc localStorage của thiết bị (không REST), nên test chạy
 * thẳng component với fakes cho cả hai API trình duyệt: Notification (switch
 * thông báo) và AudioContext/speechSynthesis (switch âm thanh + nút thử).
 *
 * I18nProvider mặc định tiếng Anh (DEFAULT_LANG = "en") — khẳng định theo chuỗi
 * của từ điển `en`, đúng những gì người dùng ở ngôn ngữ mặc định nhìn thấy.
 */
describe("features/settings/NotificationsPanel", () => {
  let attention: ReturnType<typeof installFakeAttention>;
  let notifications: ReturnType<typeof installFakeNotifications>;

  beforeEach(() => {
    attention = installFakeAttention();
    notifications = installFakeNotifications();
    notifications.setPermission("granted");
    window.localStorage.clear();
  });
  afterEach(() => {
    attention.restore();
    notifications.restore();
    resetAudioContextForTests();
    window.localStorage.clear();
  });

  it("hiển thị hai kênh đang tắt và ô tên gọi rỗng khi chưa cấu hình gì", () => {
    renderManagement(<NotificationsPanel />);

    expect(screen.getByText("Background notifications and sound")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "System notifications" })).not.toBeChecked();
    expect(screen.getByRole("switch", { name: "Called-by-name sound" })).not.toBeChecked();
    expect(screen.getByLabelText("Name to call")).toHaveValue("");
  });

  it("ô tên gọi phản chiếu tên đã lưu trong localStorage của thiết bị", () => {
    window.localStorage.setItem("bean.soundName", "Vinh Mr");
    renderManagement(<NotificationsPanel />);

    expect(screen.getByLabelText("Name to call")).toHaveValue("Vinh Mr");
  });

  it("đổi tên rồi Lưu: ghi vào localStorage và hiện trạng thái đã lưu", async () => {
    const user = userEvent.setup();
    renderManagement(<NotificationsPanel />);

    await user.type(screen.getByLabelText("Name to call"), "Vinh Mr");
    // Nút Lưu tắt khi chưa sửa gì — gõ xong mới bật được.
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(window.localStorage.getItem("bean.soundName")).toBe("Vinh Mr");
    expect(screen.getByText("Name saved")).toBeInTheDocument();
  });

  it("Thử âm thanh: lưu luôn bản nháp tên rồi phát câu với đúng tên đang gõ", async () => {
    const user = userEvent.setup();
    window.localStorage.setItem("bean.sound", "1");
    renderManagement(<NotificationsPanel />);

    await user.type(screen.getByLabelText("Name to call"), "Vinh Mr");
    await user.click(screen.getByRole("button", { name: "Test sound" }));

    // Tên nháp được lưu trước khi phát — câu đọc khớp đang gõ, không phải tên cũ.
    expect(window.localStorage.getItem("bean.soundName")).toBe("Vinh Mr");
    expect(attention.spoken).toHaveLength(1);
    expect(attention.spoken[0]?.text).toBe("Hey Vinh Mr, Bean sounds are ready.");
    // Nút thử nằm trong thao tác bấm: phát được cả khi trang đang hiển thị —
    // nên không cần stub ngữ cảnh tab ẩn ở đây.
  });

  it("bật switch âm thanh: ghi lựa chọn '1'; tắt lại thì xoá", async () => {
    const user = userEvent.setup();
    renderManagement(<NotificationsPanel />);

    const soundSwitch = screen.getByRole("switch", { name: "Called-by-name sound" });
    await user.click(soundSwitch);
    expect(window.localStorage.getItem("bean.sound")).toBe("1");
    expect(soundSwitch).toBeChecked();

    await user.click(soundSwitch);
    expect(window.localStorage.getItem("bean.sound")).toBeNull();
  });

  it("bật switch thông báo: xin quyền trình duyệt, được cấp thì ghi lựa chọn", async () => {
    const user = userEvent.setup();
    // Quyền mặc định chưa xin — switch bật thì component phải gọi requestPermission.
    notifications.setPermission("default");
    renderManagement(<NotificationsPanel />);

    await user.click(screen.getByRole("switch", { name: "System notifications" }));

    expect(notifications.requestPermission).toHaveBeenCalledTimes(1);
    expect(window.localStorage.getItem("bean.notify")).toBe("1");
  });
});
