import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HttpResponse, http } from "msw";
import { Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { renderManagement } from "@/test/management";
import { LLM_CONFIG_FIXTURE, testServer } from "@/test/server";

function renderProviders() {
  return renderManagement(
    <Routes>
      <Route path="/settings/:section" element={<SettingsPage />} />
      <Route path="*" element={<div>fallback</div>} />
    </Routes>,
    ["/settings/providers"],
  );
}

describe("SettingsPage — Providers", () => {
  it("hiển thị cấu hình runtime đã khử secret: model hiện tại, provider, trạng thái key", async () => {
    renderProviders();

    // Model hiện tại là giá trị được chọn trong combobox.
    const select = await screen.findByRole("combobox", { name: "Model" });
    expect(select).toHaveTextContent("model-a");

    // Các trường chỉ đọc nằm trong danh sách mô tả.
    expect(screen.getByText("openai_compat")).toBeInTheDocument();
    expect(screen.getByText("https://openrouter.ai/api/v1")).toBeInTheDocument();
    // Chỉ có TÊN biến môi trường; giá trị key không bao giờ xuất hiện.
    expect(screen.getByText("OPENROUTER_API_KEY")).toBeInTheDocument();
    expect(screen.getByText("Đã cấu hình trong môi trường")).toBeInTheDocument();

    // Lưu khoá khi chưa đổi: nút phải disabled.
    expect(screen.getByRole("button", { name: "Lưu" })).toBeDisabled();
  });

  it("key thiếu trong môi trường là cảnh báo destructive, không phải lỗi im lặng", async () => {
    testServer.use(
      http.get("/api/config/llm", () => HttpResponse.json({ ...LLM_CONFIG_FIXTURE, key_configured: false })),
    );
    renderProviders();

    const warning = await screen.findByText("Chưa có trong môi trường — Bean sẽ không gọi được API");
    expect(warning.className).toContain("text-destructive");
  });

  it("chọn model khác rồi Lưu: PUT đúng body, hiện thông báo thành công", async () => {
    const user = userEvent.setup();
    let sent: unknown = null;
    testServer.use(
      http.put("/api/config/llm/model", async ({ request }) => {
        sent = await request.json();
        return HttpResponse.json({ model: (sent as { model: string }).model });
      }),
    );
    renderProviders();

    const select = await screen.findByRole("combobox", { name: "Model" });
    select.focus();
    await user.keyboard("{Enter}");
    await screen.findByRole("option", { name: "model-b" });
    await user.click(screen.getByRole("option", { name: "model-b" }));

    const save = screen.getByRole("button", { name: "Lưu" });
    expect(save).toBeEnabled();
    await user.click(save);

    await screen.findByRole("status");
    expect(sent).toEqual({ model: "model-b" });
    // Combobox cập nhật theo model mới — form không giữ giá trị cũ.
    expect(screen.getByRole("combobox", { name: "Model" })).toHaveTextContent("model-b");
  });

  it("model ngoài danh sách bị server từ chối (422): hiện alert lỗi, model không đổi", async () => {
    const user = userEvent.setup();
    testServer.use(
      http.put("/api/config/llm/model", () =>
        HttpResponse.json(
          { code: "model_not_allowed", message: "model không nằm trong llm.allowed_models" },
          { status: 422 },
        ),
      ),
    );
    renderProviders();

    const select = await screen.findByRole("combobox", { name: "Model" });
    select.focus();
    await user.keyboard("{Enter}");
    await screen.findByRole("option", { name: "model-c" });
    await user.click(screen.getByRole("option", { name: "model-c" }));
    await user.click(screen.getByRole("button", { name: "Lưu" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Không lưu được. Model phải nằm trong danh sách cho phép.");
    // Không có thông báo thành công song song với lỗi.
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
  });
});
