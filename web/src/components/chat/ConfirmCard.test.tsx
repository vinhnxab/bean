import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ConfirmCard } from "@/components/chat/ConfirmCard";
import { I18nProvider } from "@/i18n";
import type { ConfirmState } from "@/store/chat";

function pendingConfirm(overrides: Partial<ConfirmState> = {}): ConfirmState {
  return {
    confirm_id: "confirm-1",
    session_id: 7,
    run_id: "run-1",
    prompt: 'run_shell --command "rm -rf /tmp/demo"',
    risk: "confirm",
    allow_session_option: true,
    timeout_seconds: 2,
    role: null,
    receivedAt: Date.now(),
    resolution: "pending",
    ...overrides,
  };
}

afterEach(() => vi.useRealTimers());

describe("ConfirmCard", () => {
  it("hiển thị hành động đầy đủ, risk và ẩn tuỳ chọn session với Dangerous", () => {
    render(
      <I18nProvider>
        <ConfirmCard
          confirm={pendingConfirm({ risk: "dangerous", allow_session_option: true })}
          onDecision={vi.fn()}
          onExpire={vi.fn()}
        />
      </I18nProvider>,
    );

    expect(screen.getByText(/rm -rf \/tmp\/demo/)).toBeInTheDocument();
    expect(screen.getByText("Dangerous")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Allow for this session" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Allow" })).not.toHaveFocus();
  });

  it("đếm ngược, khoá sau khi hết hạn và không tự focus nút cho phép", () => {
    vi.useFakeTimers();
    const onExpire = vi.fn();
    render(
      <I18nProvider>
        <ConfirmCard
          confirm={pendingConfirm({ timeout_seconds: 1 })}
          onDecision={vi.fn()}
          onExpire={onExpire}
        />
      </I18nProvider>,
    );

    expect(screen.getByRole("button", { name: "Allow" })).not.toHaveFocus();
    act(() => vi.advanceTimersByTime(1_100));
    expect(onExpire).toHaveBeenCalledWith("confirm-1");
  });

  it("khoá toàn bộ nút sau khi đã resolve", () => {
    render(
      <I18nProvider>
        <ConfirmCard
          confirm={pendingConfirm({ resolution: "allowed" })}
          onDecision={vi.fn()}
          onExpire={vi.fn()}
        />
      </I18nProvider>,
    );

    expect(screen.getByText("Allowed")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Allow" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Deny" })).not.toBeInTheDocument();
  });

  it("tô màu dòng diff: + xanh, - đỏ, header mờ (D27)", () => {
    render(
      <I18nProvider>
        <ConfirmCard
          confirm={pendingConfirm({
            prompt:
              "write_file path=ghi-chu.md — ghi đè (1 dòng → 1 dòng)\n" +
              "--- a/ghi-chu.md\n+++ b/ghi-chu.md\n" +
              "@@ -1,1 +1,1 @@\n-dòng cũ\n+dòng mới",
          })}
          onDecision={vi.fn()}
          onExpire={vi.fn()}
        />
      </I18nProvider>,
    );

    expect(screen.getByText("+dòng mới")).toHaveClass("text-live");
    expect(screen.getByText("-dòng cũ")).toHaveClass("text-alert");
    expect(screen.getByText("@@ -1,1 +1,1 @@")).toHaveClass("text-ink-muted");
    expect(screen.getByText("--- a/ghi-chu.md")).toHaveClass("text-ink-muted");
  });

  it("không tô màu phần mô tả phía trước khối diff", () => {
    render(
      <I18nProvider>
        <ConfirmCard
          confirm={pendingConfirm({
            prompt:
              "write_file path=a.md — tạo file mới (1 dòng)\n" +
              "--- a/a.md\n+++ b/a.md\n@@ -0,0 +1,1 @@\n+hello",
          })}
          onDecision={vi.fn()}
          onExpire={vi.fn()}
        />
      </I18nProvider>,
    );

    const description = screen.getByText(/tạo file mới/);
    expect(description.className).not.toContain("text-live");
    expect(description.className).not.toContain("text-alert");
    expect(screen.getByText("+hello")).toHaveClass("text-live");
  });

  it("prompt không có diff (run_shell) hiển thị nguyên văn không span màu", () => {
    render(
      <I18nProvider>
        <ConfirmCard confirm={pendingConfirm()} onDecision={vi.fn()} onExpire={vi.fn()} />
      </I18nProvider>,
    );

    const pre = screen.getByText(/rm -rf \/tmp\/demo/);
    expect(pre.querySelector("span")).toBeNull();
  });
});
