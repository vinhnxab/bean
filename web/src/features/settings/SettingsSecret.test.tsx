import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";
import type { SecretSlotDto } from "@/api/bindings";
import { ConnectionTestButton } from "@/features/settings/ConnectionTestButton";
import { SecretField } from "@/features/settings/SecretField";
import { renderManagement } from "@/test/management";
import { testServer } from "@/test/server";

const PASSWORD = "correct horse battery staple";

function slot(overrides: Partial<SecretSlotDto>): SecretSlotDto {
  return {
    name: "BILLING_API_KEY",
    label: "Billing (chi phí cloud)",
    group: "billing",
    set: false,
    source: "unset",
    shadowed_by_env: false,
    ...overrides,
  };
}

async function fillSecret(
  user: ReturnType<typeof userEvent.setup>,
  value: string,
  password: string = PASSWORD,
) {
  const valueInput = screen.getByLabelText("New value");
  await user.clear(valueInput);
  await user.type(valueInput, value);
  const passwordInput = screen.getByLabelText("Current password");
  await user.clear(passwordInput);
  await user.type(passwordInput, password);
}

describe("SecretField — ô nhập secret write-only", () => {
  it("slot chưa đặt: hiển thị 'Chưa đặt', không có ô nhập, không lộ giá trị", () => {
    renderManagement(<SecretField slot={slot({ set: false, source: "unset" })} />);

    expect(screen.getByText("Not set")).toBeInTheDocument();
    expect(screen.queryByLabelText("New value")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add" })).toBeInTheDocument();
    expect(screen.queryByText("sk-test")).not.toBeInTheDocument();
  });

  it("slot đã đặt từ secrets.env: hiển thị nguồn, nút Thay/Xoá", () => {
    renderManagement(<SecretField slot={slot({ set: true, source: "secrets_env" })} />);

    expect(screen.getByText("Set")).toBeInTheDocument();
    expect(screen.getByText("secrets.env")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Replace" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete" })).toBeInTheDocument();
  });

  it("slot bị env ghi đè: khóa nhập và hiện giải thích", () => {
    renderManagement(<SecretField slot={slot({ set: true, source: "env", shadowed_by_env: true })} />);

    expect(screen.getByText("environment variable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Replace" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Delete" })).toBeDisabled();
    expect(screen.getByText("Locked — value is overridden by environment variable.")).toBeInTheDocument();
  });

  it("lưu thành công: xóa nhập, hiện 'Đã lưu'", async () => {
    let current = slot({ set: false, source: "unset" });
    testServer.use(
      http.get("/api/secrets", () => HttpResponse.json({ slots: [current] })),
      http.put("/api/secrets/:name", async ({ request }) => {
        const body = (await request.json()) as { value: string; password: string };
        if (body.password !== PASSWORD) {
          return HttpResponse.json(
            { code: "invalid_password", message: "mật khẩu không đúng" },
            { status: 401 },
          );
        }
        current = { ...current, set: true, source: "secrets_env" };
        return HttpResponse.json({ ok: true, name: current.name, source: "secrets_env" });
      }),
    );

    const user = userEvent.setup();
    renderManagement(<SecretField slot={current} />);

    await user.click(screen.getByRole("button", { name: "Add" }));
    await fillSecret(user, "sk-test-123");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(screen.getByText("Saved.")).toBeInTheDocument());
    expect(screen.queryByLabelText("New value")).not.toBeInTheDocument();
  });

  it("sai mật khẩu: 401 hiện lỗi, không lưu", async () => {
    testServer.use(
      http.put("/api/secrets/:name", async () => {
        return HttpResponse.json(
          { code: "invalid_password", message: "mật khẩu không đúng" },
          { status: 401 },
        );
      }),
    );

    const user = userEvent.setup();
    renderManagement(<SecretField slot={slot({ set: false })} />);

    await user.click(screen.getByRole("button", { name: "Add" }));
    await fillSecret(user, "sk-test-123");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("mật khẩu không đúng"));
  });

  it("xoá thành công: về 'Chưa đặt'", async () => {
    let current = slot({ set: true, source: "secrets_env" });
    testServer.use(
      http.delete("/api/secrets/:name", async () => {
        current = { ...current, set: false, source: "unset" };
        return HttpResponse.json({ ok: true, name: current.name, source: "unset" });
      }),
    );

    const user = userEvent.setup();
    renderManagement(<SecretField slot={current} />);

    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.type(screen.getByLabelText("Current password"), PASSWORD);
    await user.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(screen.getByText("Deleted.")).toBeInTheDocument());
  });
});

describe("ConnectionTestButton — kiểm tra kết nối", () => {
  it("voice provider local: warn, không gửi yêu cầu nào lên mạng", async () => {
    testServer.use(
      http.post("/api/settings/test/voice", () =>
        HttpResponse.json({
          target: "voice",
          level: "warn",
          message: "voice: provider `local` không kiểm tra được qua HTTP",
          hint: "dùng `bean doctor` để kiểm tra binary/model whisper/piper",
        }),
      ),
    );

    const user = userEvent.setup();
    renderManagement(<ConnectionTestButton target="voice" />);

    await user.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() => expect(screen.getByText("Warning")).toBeInTheDocument());
    expect(screen.getByText("voice: provider `local` không kiểm tra được qua HTTP")).toBeInTheDocument();
  });
});
