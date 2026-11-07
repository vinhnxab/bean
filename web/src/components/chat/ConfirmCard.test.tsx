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
});
