//! Runner eval: dựng môi trường cách ly, chạy đúng một lượt qua **Router**
//! (cùng đường đi với CLI/web), thu sự kiện, chấm khẳng định, ghi metrics.
//!
//! # Vì sao qua Router mà không gọi `Agent` trực tiếp
//!
//! Router là nơi hệ thống hành xử thật: confirm đi qua `resolve_confirm`,
//! reflection chạy sau `Final`, tool bị lọc theo policy. Eval chạy đường tắt
//! thì số đo được không phản ánh hệ thống người dùng thực sự chạy — mất luôn
//! ý nghĩa "đo trước khi tối ưu".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use bean_core::{Decision, Incoming, Router, RouterDeps};
use bean_llm::{FakeProvider, LlmProvider};
use bean_memory::SqliteStore;
use bean_memory::traits::{MessageStore, SessionStore};
use bean_security::CapWorkspace;
use bean_skills::SkillCatalog;
use bean_tools::ToolRegistry;
use bean_tools::builtin::files::tool::{glob, grep, list_dir, read_file, write_file};
use bean_types::{RunEvent, config::Config, message::Message};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::recorder::UsageRecorder;
use crate::report::MetricsRecord;
use crate::task::{EvalTask, load_tasks_dir};

/// Thời gian tối đa chờ reflection tạo draft sau `Final`.
const REFLECTION_WAIT_MS: u64 = 10_000;

/// Chế độ chọn provider cho run eval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderChoice {
    /// Theo task: có `script` ⇒ fake, không thì real.
    Auto,
    /// Ép dùng FakeProvider — lỗi nếu task không có kịch bản.
    Fake,
    /// Ép dùng provider thật (bỏ qua kịch bản) — lỗi nếu thiếu key env.
    Real,
}

/// Tùy chọn vận hành của một lượt `bean eval run`.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Chọn provider.
    pub provider: ProviderChoice,
    /// Thư mục ghi `metrics.jsonl` + `trajectory/`.
    pub out_dir: PathBuf,
}

impl RunOptions {
    /// Tùy chọn mặc định: provider tự theo task, ghi metrics vào `out_dir`.
    #[must_use]
    pub fn new(out_dir: PathBuf) -> Self {
        Self {
            provider: ProviderChoice::Auto,
            out_dir,
        }
    }
}

/// Sự kiện trajectory — thứ tự tool call, confirm, text và lỗi của một run.
///
/// Đây là "băng ghi" để trả lời câu hỏi *vì sao model chọn sai tool* mà không
/// phải lần lại DB.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TrajectoryEvent {
    /// Tool bắt đầu chạy.
    ToolStart { tool: String, args_preview: String },
    /// Tool kết thúc.
    ToolEnd { tool: String, ok: bool },
    /// Agent xin xác nhận — runner tự trả lời `Allow` và ghi lại.
    Confirm { risk: String, prompt: String },
    /// Text model phát (snapshot mỗi bước).
    Text { text: String },
    /// Run kết thúc bình thường.
    Final { text: String },
    /// Run lỗi.
    Error { code: String, message: String },
}

/// Kết quả một task sau khi chạy và chấm.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskResult {
    /// Id task.
    pub task_id: String,
    /// Đạt mọi khẳng định và không có lỗi run.
    pub ok: bool,
    /// Danh sách vi phạm khẳng định (rỗng khi đạt).
    pub violations: Vec<String>,
    /// Lý do kết thúc run (`final`, `max_steps`, ...; `None` khi lỗi/timeout).
    pub end_reason: Option<String>,
    /// Text cuối của run.
    pub final_text: Option<String>,
    /// Tool đã gọi theo đúng thứ tự phát sinh.
    pub tools_called: Vec<String>,
    /// Số tool call.
    pub tool_calls: usize,
    /// Số confirm runner đã tự trả lời `Allow`.
    pub confirm_count: u32,
    /// Nội dung file workspace sau run (đường dẫn tương đối → nội dung).
    pub files: BTreeMap<String, String>,
    /// `sessions.summary` sau run (khi có — compaction đã chạy).
    pub summary: Option<String>,
    /// Số skill nháp tồn tại sau run.
    pub draft_count: usize,
    /// Tổng token vào (mọi lượt gọi LLM, gồm compaction/reflection).
    pub input_tokens: u64,
    /// Tổng token ra.
    pub output_tokens: u64,
    /// Số lượt gọi LLM.
    pub llm_calls: usize,
    /// Tên provider.
    pub provider: String,
    /// Model cấu hình.
    pub model: String,
    /// Thời gian chạy (ms).
    pub duration_ms: u128,
    /// Lỗi runner/run (khác vi phạm khẳng định).
    pub error: Option<String>,
}

/// Chạy lần lượt mọi task TOML trong thư mục; trả kết quả theo thứ tự id.
///
/// # Errors
/// Lỗi khi không nạp được task hoặc không tạo được thư mục out. Lỗi của **một**
/// task không dừng các task còn lại — nó ghi vào [`TaskResult::error`].
pub async fn run_tasks_in_dir(
    tasks_dir: &Path,
    only: Option<&str>,
    options: &RunOptions,
) -> anyhow::Result<Vec<TaskResult>> {
    let tasks = load_tasks_dir(tasks_dir)?;
    if let Some(id) = only
        && !tasks.iter().any(|task| task.id == id)
    {
        bail!("không có task nào với id `{id}`");
    }
    run_tasks(&tasks, only, options).await
}

/// Chạy một danh sách task (lọc `only` nếu có).
///
/// # Errors
/// Lỗi khi không tạo được thư mục out.
pub async fn run_tasks(
    tasks: &[EvalTask],
    only: Option<&str>,
    options: &RunOptions,
) -> anyhow::Result<Vec<TaskResult>> {
    let trajectory_dir = options.out_dir.join("trajectory");
    std::fs::create_dir_all(&trajectory_dir)
        .with_context(|| format!("tạo thư mục out {}", options.out_dir.display()))?;
    let mut results = Vec::new();
    for task in tasks {
        if let Some(id) = only
            && task.id != id
        {
            continue;
        }
        let (result, events) = run_task(task, options).await;
        let trajectory_path = trajectory_dir.join(format!("{}.json", task.id));
        let trajectory = TrajectoryFile {
            task_id: task.id.clone(),
            provider: result.provider.clone(),
            model: result.model.clone(),
            events,
            final_text: result.final_text.clone(),
        };
        write_trajectory(&trajectory_path, &trajectory);
        let record = MetricsRecord::from_result(&result);
        let metrics_path = options.out_dir.join("metrics.jsonl");
        if let Err(error) = record.append_to(&metrics_path) {
            tracing::warn!(%error, "ghi metrics thất bại");
        }
        tracing::info!(
            task = %task.id,
            ok = result.ok,
            ms = result.duration_ms as u64,
            "task eval hoàn tất"
        );
        results.push(result);
    }
    Ok(results)
}

/// Chạy đúng một task trong môi trường cách ly; trả (kết quả chấm, trajectory).
///
/// Lỗi môi trường/run đều nằm trong [`TaskResult::error`] — hàm này chỉ trả
/// `Err` khi không thể dựng môi trường eval.
pub async fn run_task(task: &EvalTask, options: &RunOptions) -> (TaskResult, Vec<TrajectoryEvent>) {
    match run_task_inner(task, options).await {
        Ok((mut result, events)) => {
            let violations = task.assert.evaluate(&result);
            result.violations = violations;
            result.ok = result.violations.is_empty() && result.error.is_none();
            (result, events)
        }
        Err(error) => (
            TaskResult {
                task_id: task.id.clone(),
                ok: false,
                error: Some(format!("{error:#}")),
                ..TaskResult::default()
            },
            Vec::new(),
        ),
    }
}

async fn run_task_inner(
    task: &EvalTask,
    options: &RunOptions,
) -> anyhow::Result<(TaskResult, Vec<TrajectoryEvent>)> {
    let started = Instant::now();
    if matches!(options.provider, ProviderChoice::Fake) && !task.llm.is_fake() {
        bail!(
            "task `{}` không có kịch bản fake — không ép chạy `--provider fake` được",
            task.id
        );
    }

    // 1. Môi trường cách ly: workspace + data + skills đều trong thư mục tạm.
    let temp = tempfile::tempdir().context("tạo thư mục tạm cho eval")?;
    let workspace_path = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).context("tạo workspace")?;
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).context("tạo data dir")?;
    for (path, content) in &task.setup.files {
        let rel = std::path::Path::new(path);
        if path.starts_with('/')
            || rel
                .components()
                .any(|c| c == std::path::Component::ParentDir)
        {
            bail!("setup file `{path}` phải là đường dẫn tương đối trong workspace");
        }
        let target = workspace_path.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("tạo thư mục cho {path}"))?;
        }
        std::fs::write(&target, content).with_context(|| format!("ghi setup file {path}"))?;
    }

    // 2. Cấu hình run — mặc định của Config + ghi đè theo task.
    let mut config = Config::default();
    config.agent.workspace = workspace_path.clone();
    if let Some(max_steps) = task.max_steps {
        config.agent.max_steps = max_steps;
    }
    if let Some(budget) = task.context_budget_tokens {
        config.agent.context_budget_tokens = budget;
    }
    config.agent.allowed_users = vec!["eval:user".into()];
    apply_task_llm(task, &mut config);
    let provider = build_provider(task, &config, options.provider)?;
    let recorder = Arc::new(UsageRecorder::new(provider));
    let model = config.llm.model.clone();
    let provider_name = recorder.name().to_string();

    // 3. Store thật (SQLite trong thư mục tạm) + registry tool file Safe/Confirm.
    let store = Arc::new(
        SqliteStore::open(&data_dir.join("bean.db"))
            .map_err(|error| anyhow::anyhow!("không mở được store eval: {error}"))?,
    );
    let workspace = Arc::new(
        CapWorkspace::open(workspace_path.clone())
            .map_err(|error| anyhow::anyhow!("không mở được workspace eval: {error}"))?,
    );
    let mut registry = ToolRegistry::with_workspace(workspace);
    for tool in [read_file(), list_dir(), glob(), grep(), write_file()] {
        registry
            .register(tool)
            .map_err(|error| anyhow::anyhow!("đăng ký tool eval thất bại: {error}"))?;
    }

    // 4. Catalog skill — chỉ dựng khi task khẳng định draft (reflection).
    let skills_root = temp.path().join("skills");
    let want_draft = task.assert.draft_created.unwrap_or(false);
    let catalog = (want_draft).then(|| {
        SkillCatalog::load_with_paths(
            std::slice::from_ref(&skills_root),
            skills_root.clone(),
            skills_root.join("_drafts"),
        )
    });

    // 5. Phiên + prefill lịch sử (kích hoạt compaction) — ghi **trước** khi
    //    submit để `resolve_session` của Router chọn đúng phiên này.
    let session = store
        .ensure_session_for_user("eval", &task.id, "eval:user", "")
        .await
        .map_err(|error| anyhow::anyhow!("tạo phiên eval thất bại: {error}"))?;
    if let Some(prefill) = &task.setup.prefill {
        if prefill.messages % 2 != 0 {
            bail!("setup.prefill.messages phải chẵn (xen kẽ user/assistant)");
        }
        for index in 0..prefill.messages / 2 {
            store
                .append(
                    session,
                    Message::user(format!("{} (lần {index})", prefill.user_text)),
                )
                .await
                .map_err(|error| anyhow::anyhow!("prefill user thất bại: {error}"))?;
            store
                .append(
                    session,
                    Message::assistant(
                        Some(format!("{} (lần {index})", prefill.assistant_text)),
                        Vec::new(),
                    ),
                )
                .await
                .map_err(|error| anyhow::anyhow!("prefill assistant thất bại: {error}"))?;
        }
    }

    // 6. Router — phụ thuộc bất biến của run (confirm, policy, reflection).
    let router = Arc::new(Router::new(RouterDeps {
        config_path: None,
        speech: None,
        default_profile: config.llm.as_profile(),
        config,
        store: store.clone(),
        registry: Arc::new(registry),
        llm: recorder.clone(),
        audit: None,
        skills_index: catalog
            .as_ref()
            .map_or_else(String::new, |catalog| catalog.index()),
        skills: catalog.clone(),
        llm_state_path: None,
    }));

    // 7. Submit và thu sự kiện — confirm tự động `Allow` (ghi đếm, không im lặng).
    let mut events = router.events();
    let _run_id = router
        .submit(Incoming::new(
            "eval",
            task.id.as_str(),
            "eval:user",
            task.prompt.clone(),
        ))
        .await
        .map_err(|error| anyhow::anyhow!("submit run eval thất bại: {error}"))?;

    let mut collector = EventCollector::new(task.timeout_seconds);
    let outcome = collector
        .drive(&router, &mut events)
        .await
        .context("chờ run eval")?;
    drop(events);
    drop(router);

    // 8. Reflection chạy bất đồng bộ sau Final — chờ draft (nếu task yêu cầu).
    let draft_count = match &catalog {
        Some(catalog) => wait_for_drafts(catalog, want_draft).await,
        None => 0,
    };

    // 9. Dữ liệu chấm: file workspace + summary (nguồn sự thật là DB).
    let files = read_workspace_files(&workspace_path)?;
    let summary = store
        .summary(session)
        .await
        .map_err(|error| anyhow::anyhow!("đọc summary thất bại: {error}"))?;

    let result = TaskResult {
        task_id: task.id.clone(),
        ok: false,
        violations: Vec::new(),
        end_reason: outcome.end_reason,
        final_text: outcome.final_text,
        tools_called: collector.tools_called.clone(),
        tool_calls: collector.tools_called.len(),
        confirm_count: collector.confirm_count,
        files,
        summary,
        draft_count,
        input_tokens: recorder.input_tokens(),
        output_tokens: recorder.output_tokens(),
        llm_calls: recorder.calls(),
        provider: provider_name,
        model,
        duration_ms: started.elapsed().as_millis(),
        error: outcome.error,
    };
    Ok((result, collector.events))
}

/// Trạng thái trả về sau khi thu sự kiện đến khi run kết thúc.
struct RunOutcome {
    end_reason: Option<String>,
    final_text: Option<String>,
    error: Option<String>,
}

/// Thu sự kiện broadcast của Router, tự trả lời confirm bằng `Allow`.
struct EventCollector {
    tools_called: Vec<String>,
    confirm_count: u32,
    events: Vec<TrajectoryEvent>,
    per_event_timeout: Duration,
}

impl EventCollector {
    fn new(timeout_seconds: u64) -> Self {
        Self {
            tools_called: Vec::new(),
            confirm_count: 0,
            events: Vec::new(),
            per_event_timeout: Duration::from_secs(timeout_seconds.max(1)),
        }
    }

    async fn drive(
        &mut self,
        router: &Router,
        events: &mut tokio::sync::broadcast::Receiver<RunEvent>,
    ) -> anyhow::Result<RunOutcome> {
        let mut outcome = RunOutcome {
            end_reason: None,
            final_text: None,
            error: None,
        };
        loop {
            let event =
                tokio::time::timeout(self.per_event_timeout, router.recv_event(events)).await;
            let event = match event {
                Ok(Some(event)) => event,
                Ok(None) => {
                    outcome.error = Some("Router đóng kênh sự kiện trước khi Final".into());
                    return Ok(outcome);
                }
                Err(_) => {
                    outcome.error = Some(format!(
                        "hết thời gian chờ {} giây — run không kết thúc",
                        self.per_event_timeout.as_secs()
                    ));
                    return Ok(outcome);
                }
            };
            match event {
                RunEvent::ToolStart {
                    tool, args_preview, ..
                } => {
                    self.tools_called.push(tool.clone());
                    self.events
                        .push(TrajectoryEvent::ToolStart { tool, args_preview });
                }
                RunEvent::ToolEnd { tool, ok, .. } => {
                    self.events.push(TrajectoryEvent::ToolEnd { tool, ok });
                }
                RunEvent::ConfirmRequest {
                    confirm_id,
                    risk,
                    prompt,
                    ..
                } => {
                    self.confirm_count += 1;
                    self.events.push(TrajectoryEvent::Confirm {
                        risk: format!("{risk:?}"),
                        prompt,
                    });
                    router
                        .resolve_confirm(confirm_id.as_str(), Decision::Allow, "eval:user")
                        .await
                        .map_err(|error| anyhow::anyhow!("resolve confirm thất bại: {error}"))?;
                }
                RunEvent::Text { text, .. } => {
                    self.events.push(TrajectoryEvent::Text { text });
                }
                RunEvent::Final { text, .. } => {
                    outcome.end_reason = Some("final".into());
                    outcome.final_text = Some(text.clone());
                    self.events.push(TrajectoryEvent::Final { text });
                    return Ok(outcome);
                }
                RunEvent::Error { code, message, .. } => {
                    outcome.error = Some(format!("[{code}] {message}"));
                    self.events.push(TrajectoryEvent::Error { code, message });
                    return Ok(outcome);
                }
                RunEvent::Queued { .. }
                | RunEvent::TextDelta { .. }
                | RunEvent::ConfirmResolved { .. }
                | RunEvent::VoiceResponse { .. } => {}
            }
        }
    }
}

/// Chờ drafts xuất hiện (reflection chạy sau Final); trả số draft hiện có.
async fn wait_for_drafts(catalog: &SkillCatalog, expected: bool) -> usize {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(REFLECTION_WAIT_MS);
    loop {
        let count = catalog.list_drafts().map_or(0, |drafts| drafts.len());
        if count > 0 || !expected || tokio::time::Instant::now() >= deadline {
            return count;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Đọc mọi file (đệ quy, bỏ thư mục) trong workspace để chấm khẳng định file.
fn read_workspace_files(root: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("đọc thư mục workspace {}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| anyhow::anyhow!("đọc entry lỗi: {error}"))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if let Ok(bytes) = std::fs::read(&path)
                && let Ok(content) = String::from_utf8(bytes)
            {
                let rel = path.strip_prefix(root).map_or_else(
                    |_| path.to_string_lossy().to_string(),
                    |p| p.to_string_lossy().to_string(),
                );
                files.insert(rel, content);
            }
        }
    }
    Ok(files)
}

/// Áp ghi đè `[llm]` của task vào cấu hình run (không đụng `bean.toml`).
fn apply_task_llm(task: &EvalTask, config: &mut Config) {
    if task.llm.is_fake() && task.llm.model.is_none() {
        // Model chỉ để ghi metrics — script fake không quan tâm model nào.
        config.llm.model = "fake-script".into();
    }
    if let Some(kind) = task.llm.provider {
        config.llm.provider = kind;
    }
    if let Some(model) = &task.llm.model {
        config.llm.model = model.clone();
    }
    if let Some(base_url) = &task.llm.base_url {
        config.llm.base_url = Some(base_url.clone());
    }
    if let Some(env_name) = &task.llm.api_key_env {
        config.llm.api_key_env = env_name.clone();
    }
}

/// Dựng provider theo task: kịch bản fake, hoặc provider thật với key từ env.
fn build_provider(
    task: &EvalTask,
    config: &Config,
    choice: ProviderChoice,
) -> anyhow::Result<Arc<dyn LlmProvider>> {
    let use_fake = match choice {
        ProviderChoice::Fake => true,
        ProviderChoice::Real => false,
        ProviderChoice::Auto => task.llm.is_fake(),
    };
    if use_fake {
        let Some(script) = &task.llm.script else {
            bail!("task `{}` không có kịch bản fake", task.id);
        };
        let provider = FakeProvider::from_json_str(script)
            .map_err(|error| anyhow::anyhow!("kịch bản fake sai: {error}"))?;
        return Ok(Arc::new(provider));
    }
    // Provider thật — key CHỈ đọc từ biến môi trường (mục 0.6): eval không đọc
    // secrets.env để không lỡ tay chạy tốn tiền khi người dùng không chủ ý.
    let key = std::env::var(&config.llm.api_key_env)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(SecretString::from);
    bean_llm::build_provider(&config.llm, key).context("dựng provider thật cho eval thất bại")
}

/// Ghi trajectory JSON (kể cả run lỗi — càng lỗi càng cần băng ghi).
fn write_trajectory(path: &Path, trajectory: &TrajectoryFile) {
    match serde_json::to_string_pretty(trajectory) {
        Ok(json) => {
            if let Err(error) = std::fs::write(path, json) {
                tracing::warn!(%error, path = %path.display(), "ghi trajectory thất bại");
            }
        }
        Err(error) => tracing::warn!(%error, "serialize trajectory thất bại"),
    }
}

/// Băng ghi đầy đủ của một run, ghi `trajectory/<task_id>.json`.
#[derive(Debug, Serialize, Deserialize)]
struct TrajectoryFile {
    /// Task tạo ra run này.
    task_id: String,
    /// Tên provider (`fake`, `openai_compat`, ...).
    provider: String,
    /// Model cấu hình.
    model: String,
    /// Chuỗi sự kiện theo thứ tự thời gian.
    events: Vec<TrajectoryEvent>,
    /// Text cuối nếu run kết thúc bình thường.
    final_text: Option<String>,
}
