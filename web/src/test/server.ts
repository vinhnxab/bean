import { HttpResponse, http } from "msw";
import { setupServer } from "msw/node";

export const testServer = setupServer(
  http.get("/api/auth/me", () => HttpResponse.json({ user_id: "web:admin" })),
  http.get("/api/sessions", () => HttpResponse.json({ sessions: [] })),
  http.get("/api/sessions/:id/messages", () => HttpResponse.json({ messages: [] })),
  http.post("/api/sessions", () =>
    HttpResponse.json(
      {
        id: 1,
        channel: "web",
        chat_id: "web:admin",
        user_id: "web:admin",
        title: "",
        archived: false,
        created_at: new Date(0).toISOString(),
        updated_at: new Date(0).toISOString(),
      },
      { status: 201 },
    ),
  ),
  http.get("/api/status", () =>
    HttpResponse.json({
      version: "0.1.0",
      model: "fake-model",
      max_steps: 25,
      daily_token_budget: 1000,
      tokens_used: 0,
      uptime_seconds: 0,
      channels: ["web"],
    }),
  ),
  http.get("/api/memory/files/:name", ({ params }) =>
    HttpResponse.json({ name: String(params.name), content: "" }),
  ),
  http.get("/api/memories", () => HttpResponse.json({ memories: [] })),
  http.get("/api/skills", () => HttpResponse.json({ skills: [] })),
  http.get("/api/skills/drafts", () => HttpResponse.json({ drafts: [] })),
  http.get("/api/tasks", () => HttpResponse.json({ tasks: [] })),
  http.get("/api/audit", () => HttpResponse.json({ entries: [] })),
  // `/api/agents` mặc định trả danh sách rỗng: phần lớn test chỉ cần "không lỗi".
  // Test HUB dùng `testServer.use(...)` để ghi đè theo từng kịch bản.
  http.get("/api/agents", () => HttpResponse.json({ agents: [], viewer_role: "admin" })),
  // Ba endpoint "capability" cũng mặc định rỗng vì nhiều màn (chat, hub) tình cờ
  // gọi chúng qua widget chung. Test riêng của từng màn ghi đè theo kịch bản.
  http.get("/api/tools", () => HttpResponse.json(TOOLS_FIXTURE_EMPTY)),
  http.get("/api/mcp", () => HttpResponse.json({ servers: [] })),
  http.get("/api/usage", () => HttpResponse.json({ days: [], daily_token_budget: 1000, today_tokens: 0 })),
  http.get("/api/system", () => HttpResponse.json(SYSTEM_FIXTURE)),
  // Màn Cài đặt (providers). Model khớp `allowed_models[0]` để test vừa mở trang
  // không lọt nhánh "dirty". Test nào cần kịch bản khác thì ghi đè theo use().
  http.get("/api/config/llm", () => HttpResponse.json(LLM_CONFIG_FIXTURE)),
  http.put("/api/config/llm/model", async ({ request }) => {
    const body = (await request.json()) as { model: string };
    return HttpResponse.json({ model: body.model });
  }),
  http.put("/api/config/llm/provider", async ({ request }) => {
    const body = (await request.json()) as { profile: string; model?: string };
    // Trả snapshot hợp lệ: profile vừa chọn thành active (fixture mặc định có 2 profile).
    const target = LLM_CONFIG_FIXTURE.providers.find((provider) => provider.name === body.profile) ?? null;
    if (!target) {
      return HttpResponse.json(
        { code: "profile_not_found", message: "profile không tồn tại trong cấu hình" },
        { status: 422 },
      );
    }
    return HttpResponse.json({
      ...LLM_CONFIG_FIXTURE,
      provider: target.provider,
      model: body.model ?? target.model,
      allowed_models: target.allowed_models,
      api_key_env: target.api_key_env,
      base_url: target.base_url,
      key_configured: target.key_configured,
      providers: LLM_CONFIG_FIXTURE.providers.map((provider) => ({
        ...provider,
        active: provider.name === body.profile,
      })),
    });
  }),
  // CRUD profile (giai đoạn 3). Handler dựa trên fixture tĩnh (không giữ state
  // giữa request): test nào cần kịch bản lỗi/đặc biệt sẽ `testServer.use(...)`
  // ghi đè riêng.
  http.post("/api/config/llm/providers", async ({ request }) => {
    const body = (await request.json()) as {
      name: string;
      provider: string;
      model: string;
      allowed_models?: string[];
      api_key_env?: string;
      api_key?: string | null;
      base_url?: string | null;
      max_tokens?: number | null;
    };
    if (body.name === "default" || LLM_CONFIG_FIXTURE.providers.some((entry) => entry.name === body.name)) {
      return HttpResponse.json(
        { code: "profile_exists", message: "đã có profile với tên này" },
        { status: 409 },
      );
    }
    return HttpResponse.json(
      {
        ...LLM_CONFIG_FIXTURE,
        providers: [
          ...LLM_CONFIG_FIXTURE.providers,
          {
            name: body.name,
            provider: body.provider,
            model: body.model,
            allowed_models: body.allowed_models ?? [],
            base_url: body.base_url ?? null,
            max_tokens: body.max_tokens ?? 4096,
            api_key_env: body.api_key_env ?? "",
            key_configured: (body.api_key_env ?? "") !== "" || (body.api_key ?? "") !== "",
            api_key_hint: body.api_key ? `••••${body.api_key.slice(-4)}` : null,
            active: false,
          },
        ],
      },
      { status: 201 },
    );
  }),
  http.put("/api/config/llm/providers/:name", async ({ request, params }) => {
    const name = String(params.name);
    const body = (await request.json()) as {
      provider: string;
      model: string;
      allowed_models?: string[];
      api_key_env?: string;
      api_key?: string | null;
      base_url?: string | null;
      max_tokens?: number | null;
    };
    const target = LLM_CONFIG_FIXTURE.providers.find((entry) => entry.name === name) ?? null;
    if (name === "default") {
      return HttpResponse.json(
        { code: "default_readonly", message: "profile 'default' là ánh xạ của [llm]" },
        { status: 422 },
      );
    }
    if (!target) {
      return HttpResponse.json(
        { code: "profile_not_found", message: "profile không tồn tại trong cấu hình" },
        { status: 422 },
      );
    }
    const updatedProvider = {
      ...target,
      provider: body.provider,
      model: body.model,
      allowed_models: body.allowed_models ?? [],
      api_key_env: body.api_key_env ?? "",
      api_key_hint:
        body.api_key && body.api_key !== ""
          ? `••••${body.api_key.slice(-4)}`
          : body.api_key === ""
            ? null
            : (target.api_key_hint ?? null),
      base_url: body.base_url ?? null,
    };
    return HttpResponse.json({
      // Profile đang active được sửa → runtime đổi theo (model, allowed_models…).
      ...(target.active ? updatedProvider : LLM_CONFIG_FIXTURE),
      providers: LLM_CONFIG_FIXTURE.providers.map((entry) => (entry.name === name ? updatedProvider : entry)),
    });
  }),
  http.delete("/api/config/llm/providers/:name", ({ params }) => {
    const name = String(params.name);
    const target = LLM_CONFIG_FIXTURE.providers.find((entry) => entry.name === name) ?? null;
    if (!target) {
      return HttpResponse.json(
        { code: "profile_not_found", message: "profile không tồn tại trong cấu hình" },
        { status: 422 },
      );
    }
    if (target.active) {
      return HttpResponse.json(
        { code: "profile_active", message: "profile đang active — hãy chuyển sang profile khác trước" },
        { status: 409 },
      );
    }
    return HttpResponse.json({
      ...LLM_CONFIG_FIXTURE,
      providers: LLM_CONFIG_FIXTURE.providers.filter((entry) => entry.name !== name),
    });
  }),
  // (M27) Mặc định voice TẮT: màn chat luôn hỏi trạng thái này để quyết định có hiện
  // nút mic. Test nào cần bật thì `testServer.use(...)` ghi đè. `configured: true`
  // mô phỏng `[voice]` đã khai báo đủ trong bean.toml (provider dựng được lúc khởi động).
  http.get("/api/voice", () =>
    HttpResponse.json({
      enabled: false,
      configured: true,
      provider: "local",
      language: "vi",
      max_speak_chars: 400,
    }),
  ),
);

/**
 * Fixture `/api/config/llm` — đã khử secret: không có giá trị key nào ở đây.
 * Hai profile (`default` active + `ollama`) để test hot-swap provider.
 */
export const LLM_CONFIG_FIXTURE = {
  provider: "openai_compat",
  model: "model-a",
  allowed_models: ["model-a", "model-b", "model-c"],
  api_key_env: "OPENROUTER_API_KEY",
  base_url: "https://openrouter.ai/api/v1",
  max_tokens: 16384,
  key_configured: true,
  providers: [
    {
      name: "default",
      provider: "openai_compat",
      model: "model-a",
      allowed_models: ["model-a", "model-b", "model-c"],
      base_url: "https://openrouter.ai/api/v1",
      max_tokens: 16384,
      api_key_env: "OPENROUTER_API_KEY",
      key_configured: true,
      api_key_hint: null,
      active: true,
    },
    {
      name: "ollama",
      provider: "openai_compat",
      model: "qwen3:14b",
      allowed_models: ["qwen3:14b", "qwen3:32b"],
      base_url: "http://localhost:11434/v1",
      max_tokens: 4096,
      api_key_env: "",
      key_configured: true,
      api_key_hint: null,
      active: false,
    },
  ],
} as const;

/**
 * Dữ liệu agent cho test — phản ánh đúng hệ trong `bean.example.toml`:
 * Manager điều phối, QA review (four-eyes), Security-scan có đường cảnh báo riêng.
 */
export const FULL_AGENTS = [
  { role: "developer", status: "working", summary: "đang thực hiện lượt", risks: [], relation: "manages" },
  { role: "qa", status: "idle", summary: "không có việc nào đang chạy", risks: [], relation: "reviews" },
  {
    role: "marketing",
    status: "awaiting_you",
    summary: "1 hành động đang chờ bạn duyệt",
    risks: ["marketing_publish"],
    relation: "manages",
  },
  {
    role: "security-scan",
    status: "idle",
    summary: "không có việc nào đang chạy",
    risks: [],
    relation: "alerts_directly",
  },
] as const;

/** Fixture `/api/agents` cho một tập agent tuỳ ý. */
export function agentsHandler(agents: unknown[], viewerRole = "admin") {
  return http.get("/api/agents", () => HttpResponse.json({ agents, viewer_role: viewerRole }));
}

/** `/api/tools` rỗng — RBAC từ chối hết cũng ra hình dạng này. */
export const TOOLS_FIXTURE_EMPTY = {
  tools: [],
  total: 0,
  viewer_role: "admin",
  rbac_enabled: false,
} as const;

/** Ba tool đại diện cho ba mức rủi ro + một tool từ MCP server. */
export const TOOLS_FIXTURE = {
  tools: [
    {
      name: "read_file",
      description: "Đọc một file trong workspace.",
      risk: "safe",
      source: "builtin",
      mcp_server: null,
      required_tags: [],
      extra_tags: [],
      untrusted: true,
      enabled: true,
    },
    {
      name: "run_shell",
      description: "Chạy lệnh shell trong sandbox.",
      risk: "confirm",
      source: "builtin",
      mcp_server: null,
      required_tags: ["dev-write"],
      extra_tags: [],
      untrusted: true,
      enabled: false,
    },
    {
      name: "mcp__siem__cve_lookup",
      description: "Tra CVE từ server SIEM nội bộ.",
      risk: "confirm",
      source: "mcp",
      mcp_server: "siem",
      required_tags: ["infra-read"],
      extra_tags: [],
      untrusted: true,
      enabled: true,
    },
  ],
  total: 5,
  viewer_role: "finance-readonly",
  rbac_enabled: true,
} as const;

/** Một server đã discovery thành công, một server chết. */
export const MCP_FIXTURE = {
  servers: [
    {
      name: "siem",
      command: "docker",
      args: ["run", "--rm", "siem-mcp"],
      trusted: false,
      call_timeout_seconds: 300,
      required_tags: ["infra-read"],
      tool_count: 4,
      connected: true,
    },
    {
      name: "weather",
      command: "/usr/local/bin/weather-mcp",
      args: [],
      trusted: true,
      call_timeout_seconds: null,
      required_tags: [],
      tool_count: 0,
      connected: false,
    },
  ],
} as const;

/**
 * `/api/system` — cùng hình dạng với `bean.toml` mặc định, đã khử secret:
 * không có API key, bot token hay `env` của MCP server trong fixture này.
 */
export const SYSTEM_FIXTURE = {
  agent_name: "Bean",
  workspace: "/home/bean/workspace",
  timezone: "Asia/Ho_Chi_Minh",
  provider: "anthropic",
  model: "fake-model",
  api_key_env: "ANTHROPIC_API_KEY",
  max_tokens: 4096,
  context_budget_tokens: 100_000,
  max_steps: 25,
  tool_timeout_seconds: 60,
  tool_groups: ["files", "shell", "web", "memory", "skills", "schedule"],
  projects: ["default"],
  roles: [{ name: "admin", tool_tags: ["*"], forbid_tags: [], allowed_tool_tags: [] }],
  rbac_enabled: false,
  sandbox: {
    mode: "docker",
    image: "bean-sandbox:latest",
    network: false,
    memory: "512m",
    cpus: 1,
    pids_limit: 64,
    timeout_seconds: 60,
  },
  web: { enabled: true, bind: "127.0.0.1:7878", allow_remote: false, session_ttl_hours: 168 },
  telegram: { enabled: false, allowed_users: 0, rate_limit_per_minute: 20 },
  learning_enabled: true,
  mcp_server_enabled: false,
  mcp_clients: 0,
  browser_enabled: false,
} as const;

/** Fixture `/api/usage` với `days` là `[{ day, total }]` (token ra + vào bằng nhau). */
export function usageFixture(days: { day: string; total: number }[]) {
  return {
    days: days.map(({ day, total }) => ({
      day,
      input_tokens: Math.floor(total / 2),
      output_tokens: total - Math.floor(total / 2),
      total_tokens: total,
    })),
    daily_token_budget: 1_000_000,
    today_tokens: days[days.length - 1]?.total ?? 0,
  };
}
