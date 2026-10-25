import { screen, waitFor, within } from "@testing-library/react";
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
    // Trạng thái key xuất hiện 2 lần: card profile mặc định + panel thông tin.
    expect(screen.getAllByText("Đã cấu hình trong môi trường")).toHaveLength(2);

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
    expect(alert).toHaveTextContent("Không lưu được. Kiểm tra model nằm trong danh sách");
    // Không có thông báo thành công song song với lỗi.
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
  });

  it("hiển thị danh sách profile: profile active có badge, profile khác có trạng thái key", async () => {
    renderProviders();

    const group = await screen.findByRole("radiogroup", { name: "Nhà cung cấp" });
    expect(group).toBeInTheDocument();
    // Profile mặc định active — radio được chọn và có badge "Đang dùng".
    const defaultRadio = screen.getByRole("radio", { name: "default" });
    expect(defaultRadio).toBeChecked();
    expect(screen.getByText("Đang dùng")).toBeInTheDocument();
    // Profile ollama: base URL tự host + không cần key.
    expect(screen.getByText("openai_compat · http://localhost:11434/v1")).toBeInTheDocument();
    expect(screen.getByText("Không cần API key (server tự host)")).toBeInTheDocument();
    // Lưu khoá khi chưa đổi gì.
    expect(screen.getByRole("button", { name: "Lưu" })).toBeDisabled();
  });

  it("chọn profile khác rồi Lưu: PUT /provider đúng body, UI phản chiếu snapshot mới", async () => {
    const user = userEvent.setup();
    let sent: unknown = null;
    testServer.use(
      http.put("/api/config/llm/provider", async ({ request }) => {
        sent = await request.json();
        const body = sent as { profile: string; model?: string };
        const target = LLM_CONFIG_FIXTURE.providers.find((p) => p.name === body.profile);
        if (!target) throw new Error("profile không tồn tại");
        return HttpResponse.json({
          ...LLM_CONFIG_FIXTURE,
          provider: target.provider,
          model: body.model ?? target.model,
          allowed_models: target.allowed_models,
          api_key_env: target.api_key_env,
          base_url: target.base_url,
          key_configured: target.key_configured,
          providers: LLM_CONFIG_FIXTURE.providers.map((p) => ({
            ...p,
            active: p.name === body.profile,
          })),
        });
      }),
    );
    renderProviders();

    // Chọn profile ollama — danh sách model chuyển sang allowed_models của profile.
    await screen.findByRole("radiogroup", { name: "Nhà cung cấp" });
    await user.click(screen.getByRole("radio", { name: "ollama" }));
    const select = screen.getByRole("combobox", { name: "Model" });
    select.focus();
    await user.keyboard("{Enter}");
    // Danh sách model chuyển sang allowed_models của profile (không còn model cũ).
    expect(await screen.findByRole("option", { name: "qwen3:14b" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "model-a" })).not.toBeInTheDocument();
    await user.keyboard("{Escape}");

    await user.click(screen.getByRole("button", { name: "Lưu" }));

    await screen.findByRole("status");
    // Không chọn model: gửi null = dùng model mặc định của profile (serde → None).
    expect(sent).toEqual({ profile: "ollama", model: null });
    // Badge chuyển sang ollama — UI phản chiếu snapshot server trả về.
    expect(screen.getByRole("radio", { name: "ollama" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "default" })).not.toBeChecked();
    const badges = screen.getAllByText("Đang dùng");
    expect(badges).toHaveLength(1);
  });

  it("chọn profile kèm model khác: PUT /provider gửi cả model", async () => {
    const user = userEvent.setup();
    let sent: unknown = null;
    testServer.use(
      http.put("/api/config/llm/provider", async ({ request }) => {
        sent = await request.json();
        const body = sent as { profile: string; model?: string };
        const target = LLM_CONFIG_FIXTURE.providers.find((p) => p.name === body.profile);
        if (!target) throw new Error("profile không tồn tại");
        return HttpResponse.json({
          ...LLM_CONFIG_FIXTURE,
          model: body.model ?? target.model,
          providers: LLM_CONFIG_FIXTURE.providers.map((p) => ({
            ...p,
            active: p.name === body.profile,
          })),
        });
      }),
    );
    renderProviders();

    await screen.findByRole("radiogroup", { name: "Nhà cung cấp" });
    await user.click(screen.getByRole("radio", { name: "ollama" }));
    const select = screen.getByRole("combobox", { name: "Model" });
    select.focus();
    await user.keyboard("{Enter}");
    await screen.findByRole("option", { name: "qwen3:32b" });
    await user.click(screen.getByRole("option", { name: "qwen3:32b" }));
    await user.click(screen.getByRole("button", { name: "Lưu" }));

    await screen.findByRole("status");
    expect(sent).toEqual({ profile: "ollama", model: "qwen3:32b" });
  });

  it("switch bị server từ chối (provider_build_failed khi thiếu key): alert lỗi, badge không đổi", async () => {
    const user = userEvent.setup();
    testServer.use(
      http.put("/api/config/llm/provider", () =>
        HttpResponse.json(
          { code: "provider_build_failed", message: "không dựng được provider" },
          { status: 422 },
        ),
      ),
    );
    renderProviders();

    await screen.findByRole("radiogroup", { name: "Nhà cung cấp" });
    await user.click(screen.getByRole("radio", { name: "ollama" }));
    await user.click(screen.getByRole("button", { name: "Lưu" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Không lưu được. Kiểm tra model nằm trong danh sách");
    // Form giữ lựa chọn của người dùng để sửa lại; badge "Đang dùng" vẫn trên profile
    // mặc định vì snapshot server không đổi sau lỗi.
    expect(screen.getByRole("radio", { name: "ollama" })).toBeChecked();
    expect(screen.getAllByText("Đang dùng")).toHaveLength(1);
  });

  it("profile thiếu key trong môi trường: cảnh báo destructive ngay trên card", async () => {
    testServer.use(
      http.get("/api/config/llm", () =>
        HttpResponse.json({
          ...LLM_CONFIG_FIXTURE,
          providers: LLM_CONFIG_FIXTURE.providers.map((p) =>
            p.name === "ollama" ? { ...p, key_configured: false, api_key_env: "OLLAMA_KEY" } : p,
          ),
        }),
      ),
    );
    renderProviders();

    // Chỉ card ollama thiếu key; panel thông tin (profile active) vẫn key OK.
    const warnings = await screen.findAllByText("Chưa có trong môi trường — Bean sẽ không gọi được API");
    expect(warnings).toHaveLength(1);
    expect(warnings[0].className).toContain("text-destructive");
  });
});

describe("SettingsPage — Providers CRUD", () => {
  it("profile mặc định không có nút sửa/xoá; có nút thêm profile", async () => {
    renderProviders();

    await screen.findByRole("radiogroup", { name: "Nhà cung cấp" });
    expect(screen.getByRole("button", { name: "Thêm profile" })).toBeInTheDocument();
    // Chỉ card ollama có nút sửa/xoá — default là ánh xạ của [llm], read-only.
    expect(screen.getAllByRole("button", { name: "Sửa profile" })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Xoá profile" })).toHaveLength(1);
  });

  it("thêm profile: POST đúng body, card mới xuất hiện, thông báo thành công", async () => {
    const user = userEvent.setup();
    let sent: unknown = null;
    testServer.use(
      http.post("/api/config/llm/providers", async ({ request }) => {
        sent = await request.json();
        const body = sent as { name: string; provider: string; model: string };
        return HttpResponse.json(
          {
            ...LLM_CONFIG_FIXTURE,
            providers: [
              ...LLM_CONFIG_FIXTURE.providers,
              {
                name: body.name,
                provider: body.provider,
                model: body.model,
                allowed_models: [body.model],
                base_url: null,
                api_key_env: "OPENROUTER_API_KEY",
                key_configured: true,
                api_key_hint: null,
                active: false,
              },
            ],
          },
          { status: 201 },
        );
      }),
    );
    renderProviders();

    await user.click(await screen.findByRole("button", { name: "Thêm profile" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Tên profile"), "openrouter");
    await user.type(within(dialog).getByLabelText("Model"), "anthropic/claude-sonnet-5");
    await user.type(within(dialog).getByLabelText("Biến môi trường API key"), "OPENROUTER_API_KEY");
    await user.click(within(dialog).getByRole("button", { name: "Lưu" }));

    await waitFor(() => expect(sent).not.toBeNull());
    expect(sent).toEqual({
      name: "openrouter",
      provider: "anthropic",
      model: "anthropic/claude-sonnet-5",
      allowed_models: [],
      api_key_env: "OPENROUTER_API_KEY",
      api_key: null,
      base_url: null,
      max_tokens: null,
    });
    // Dialog đóng sau thành công; snapshot mới có card openrouter (radio).
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(await screen.findByRole("radio", { name: "openrouter" })).toBeInTheDocument();
    expect(screen.getByText("Đã thêm profile vào bean.toml.")).toBeInTheDocument();
  });

  it("thêm profile với key dán trực tiếp: POST chứa api_key, card hiển thị hint 4 ký tự cuối", async () => {
    const user = userEvent.setup();
    let sent: unknown = null;
    testServer.use(
      http.post("/api/config/llm/providers", async ({ request }) => {
        sent = await request.json();
        const body = sent as { name: string; api_key: string | null };
        return HttpResponse.json(
          {
            ...LLM_CONFIG_FIXTURE,
            providers: [
              ...LLM_CONFIG_FIXTURE.providers,
              {
                name: body.name,
                provider: "openai_compat",
                model: "m",
                allowed_models: ["m"],
                base_url: null,
                api_key_env: "",
                key_configured: true,
                api_key_hint: body.api_key ? `••••${body.api_key.slice(-4)}` : null,
                active: false,
              },
            ],
          },
          { status: 201 },
        );
      }),
    );
    renderProviders();

    await user.click(await screen.findByRole("button", { name: "Thêm profile" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Tên profile"), "paid-key");
    await user.type(within(dialog).getByLabelText("Model"), "m");
    const keyInput = within(dialog).getByLabelText("API key");
    expect(keyInput).toHaveAttribute("type", "password");
    await user.type(keyInput, "sk-test-abcd");
    await user.click(within(dialog).getByRole("button", { name: "Lưu" }));

    await waitFor(() => expect(sent).not.toBeNull());
    expect(sent).toMatchObject({ api_key: "sk-test-abcd" });
    // Key nguyên vẹn không bao giờ hiển thị lại — chỉ hint 4 ký tự cuối.
    expect(await screen.findByText("Đã lưu trực tiếp trong bean.toml (••••abcd)")).toBeInTheDocument();
    expect(screen.queryByText("sk-test-abcd")).not.toBeInTheDocument();
  });

  it("sửa profile: PUT đúng tên trên đường dẫn, body không có trường tên", async () => {
    const user = userEvent.setup();
    let sent: unknown = null;
    let path = "";
    testServer.use(
      http.put("/api/config/llm/providers/:name", async ({ request, params }) => {
        path = String(params.name);
        sent = await request.json();
        return HttpResponse.json({
          ...LLM_CONFIG_FIXTURE,
          providers: LLM_CONFIG_FIXTURE.providers.map((p) =>
            p.name === "ollama" ? { ...p, model: "qwen3:32b", allowed_models: ["qwen3:32b"] } : p,
          ),
        });
      }),
    );
    renderProviders();

    await user.click(await screen.findByRole("button", { name: "Sửa profile" }));
    const dialog = await screen.findByRole("dialog");
    // Không có ô "Tên profile" khi sửa — tên là định danh lấy từ đường dẫn.
    expect(within(dialog).queryByLabelText("Tên profile")).not.toBeInTheDocument();
    const modelInput = within(dialog).getByLabelText("Model");
    await user.clear(modelInput);
    await user.type(modelInput, "qwen3:32b");
    await user.click(within(dialog).getByRole("button", { name: "Lưu" }));

    await waitFor(() => expect(sent).not.toBeNull());
    expect(path).toBe("ollama");
    expect(sent).toMatchObject({ provider: "openai_compat", model: "qwen3:32b", api_key: null });
    expect(await screen.findByText("Đã cập nhật profile trong bean.toml.")).toBeInTheDocument();
  });

  it("sửa profile có key đã lưu: ô trống giữ nguyên key (Keep), placeholder hiện hint", async () => {
    const user = userEvent.setup();
    let sent: unknown = null;
    testServer.use(
      http.get("/api/config/llm", () =>
        HttpResponse.json({
          ...LLM_CONFIG_FIXTURE,
          providers: LLM_CONFIG_FIXTURE.providers.map((p) =>
            p.name === "ollama" ? { ...p, api_key_hint: "••••abcd" } : p,
          ),
        }),
      ),
      http.put("/api/config/llm/providers/:name", async ({ request }) => {
        sent = await request.json();
        return HttpResponse.json(LLM_CONFIG_FIXTURE);
      }),
    );
    renderProviders();

    await user.click(await screen.findByRole("button", { name: "Sửa profile" }));
    const dialog = await screen.findByRole("dialog");
    // Placeholder phản chiếu key đã lưu — người dùng biết để trống là giữ nguyên.
    expect(within(dialog).getByLabelText("API key")).toHaveAttribute(
      "placeholder",
      "Đã lưu ••••abcd — để trống giữ nguyên",
    );
    expect(within(dialog).getByRole("button", { name: "Xoá key đã lưu" })).toBeInTheDocument();

    // Submit không dán gì ⇒ api_key: null = Keep (không xoá key đã lưu).
    await user.click(within(dialog).getByRole("button", { name: "Lưu" }));
    await waitFor(() => expect(sent).not.toBeNull());
    expect(sent).toMatchObject({ api_key: null });
  });

  it("nút Xoá key đã lưu: vô hiệu ô nhập, submit gửi chuỗi rỗng (Clear)", async () => {
    const user = userEvent.setup();
    let sent: unknown = null;
    testServer.use(
      http.get("/api/config/llm", () =>
        HttpResponse.json({
          ...LLM_CONFIG_FIXTURE,
          providers: LLM_CONFIG_FIXTURE.providers.map((p) =>
            p.name === "ollama" ? { ...p, api_key_hint: "••••abcd" } : p,
          ),
        }),
      ),
      http.put("/api/config/llm/providers/:name", async ({ request }) => {
        sent = await request.json();
        return HttpResponse.json(LLM_CONFIG_FIXTURE);
      }),
    );
    renderProviders();

    await user.click(await screen.findByRole("button", { name: "Sửa profile" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Xoá key đã lưu" }));

    const keyInput = within(dialog).getByLabelText("API key");
    expect(keyInput).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "Lưu" }));

    await waitFor(() => expect(sent).not.toBeNull());
    expect(sent).toMatchObject({ api_key: "" });
  });

  it("xoá profile không active: DELETE được gọi, card biến mất", async () => {
    const user = userEvent.setup();
    let deleted = "";
    testServer.use(
      http.delete("/api/config/llm/providers/:name", ({ params }) => {
        deleted = String(params.name);
        return HttpResponse.json({
          ...LLM_CONFIG_FIXTURE,
          providers: LLM_CONFIG_FIXTURE.providers.filter((p) => p.name !== "ollama"),
        });
      }),
    );
    renderProviders();

    await user.click(await screen.findByRole("button", { name: "Xoá profile" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("Xoá profile?");
    await user.click(within(dialog).getByRole("button", { name: "Xoá" }));

    expect(await screen.findByText("Đã xoá profile khỏi bean.toml.")).toBeInTheDocument();
    expect(deleted).toBe("ollama");
    expect(screen.queryByRole("radio", { name: "ollama" })).not.toBeInTheDocument();
  });

  it("xoá profile đang active bị server chặn 409: alert lỗi, dialog vẫn mở", async () => {
    const user = userEvent.setup();
    testServer.use(
      http.delete("/api/config/llm/providers/:name", () =>
        HttpResponse.json(
          { code: "profile_active", message: "profile đang active — hãy chuyển sang profile khác trước" },
          { status: 409 },
        ),
      ),
    );
    renderProviders();

    await user.click(await screen.findByRole("button", { name: "Xoá profile" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Xoá" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "profile đang active — hãy chuyển sang profile khác trước",
    );
    // Confirm dialog vẫn mở (onSuccess không chạy) — người dùng có thể Huỷ.
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});

/**
 * Section Giọng nói: PATCH /api/voice bật/tắt lúc chạy (M27+). Fixture mặc định
 * đã `configured: true, enabled: false` — các test ghi đè bằng `testServer.use`.
 */
function renderVoice() {
  return renderManagement(
    <Routes>
      <Route path="/settings/:section" element={<SettingsPage />} />
      <Route path="*" element={<div>fallback</div>} />
    </Routes>,
    ["/settings/voice"],
  );
}

describe("SettingsPage — Voice", () => {
  it("hiển thị trạng thái: provider, ngôn ngữ, switch phản chiếu enabled", async () => {
    renderVoice();

    expect(await screen.findByText("Giọng nói (push-to-talk)")).toBeInTheDocument();
    expect(screen.getByText("local")).toBeInTheDocument();
    expect(screen.getByText("vi")).toBeInTheDocument();
    // Fixture mặc định: configured, đang tắt.
    expect(screen.getByRole("switch", { name: "Bật/tắt giọng nói" })).not.toBeChecked();
    expect(screen.getByText("Đang tắt")).toBeInTheDocument();
  });

  it("bật voice: PATCH đúng body, switch và trạng thái cập nhật theo snapshot server", async () => {
    const user = userEvent.setup();
    let sent: unknown = null;
    testServer.use(
      http.patch("/api/voice", async ({ request }) => {
        sent = await request.json();
        return HttpResponse.json({
          enabled: true,
          configured: true,
          provider: "local",
          language: "vi",
          max_speak_chars: 400,
        });
      }),
    );
    renderVoice();

    const toggle = await screen.findByRole("switch", { name: "Bật/tắt giọng nói" });
    await user.click(toggle);

    await waitFor(() => expect(sent).toEqual({ enabled: true }));
    expect(screen.getByRole("switch", { name: "Bật/tắt giọng nói" })).toBeChecked();
    expect(screen.getByText("Đang bật")).toBeInTheDocument();
  });

  it("server từ chối (422 voice_state_unwritable): alert lỗi, switch giữ nguyên", async () => {
    const user = userEvent.setup();
    testServer.use(
      http.patch("/api/voice", () =>
        HttpResponse.json(
          { code: "voice_state_unwritable", message: "không ghi được file trạng thái voice" },
          { status: 422 },
        ),
      ),
    );
    renderVoice();

    const toggle = await screen.findByRole("switch", { name: "Bật/tắt giọng nói" });
    await user.click(toggle);

    expect(await screen.findByRole("alert")).toHaveTextContent("không ghi được file trạng thái voice");
    expect(screen.getByRole("switch", { name: "Bật/tắt giọng nói" })).not.toBeChecked();
  });

  it("chưa cấu hình [voice]: switch bị khoá kèm hướng dẫn, không gửi PATCH", async () => {
    const user = userEvent.setup();
    let patchCount = 0;
    testServer.use(
      http.get("/api/voice", () =>
        HttpResponse.json({
          enabled: false,
          configured: false,
          provider: "local",
          language: "vi",
          max_speak_chars: 400,
        }),
      ),
      http.patch("/api/voice", () => {
        patchCount += 1;
        return HttpResponse.json({ code: "voice_not_configured", message: "chưa cấu hình" }, { status: 409 });
      }),
    );
    renderVoice();

    const toggle = await screen.findByRole("switch", { name: "Bật/tắt giọng nói" });
    expect(toggle).toBeDisabled();
    expect(screen.getByText("Chưa cấu hình")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Chưa khai báo đủ phần [voice] trong bean.toml — hoàn thiện cấu hình rồi khởi động lại Bean để bật được từ đây.",
      ),
    ).toBeInTheDocument();
    // disabled ⇒ không thể click; guard phòng hờ: PATCH không bao giờ được gọi.
    await user.click(toggle);
    await waitFor(() => expect(patchCount).toBe(0));
  });

  it("provider api: hiện cảnh báo privacy bản ghi âm gửi lên dịch vụ ngoài", async () => {
    testServer.use(
      http.get("/api/voice", () =>
        HttpResponse.json({
          enabled: true,
          configured: true,
          provider: "api",
          language: "vi",
          max_speak_chars: 400,
        }),
      ),
    );
    renderVoice();

    expect(await screen.findByRole("switch", { name: "Bật/tắt giọng nói" })).toBeChecked();
    expect(
      screen.getByText("Bản ghi âm sẽ được gửi lên dịch vụ giọng nói bên ngoài để nhận dạng/phát âm thanh."),
    ).toBeInTheDocument();
  });
});
