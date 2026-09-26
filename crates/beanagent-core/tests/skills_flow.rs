//! Regression M6: model nạp skill trước khi trả lời theo hướng dẫn.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beanagent_core::run_io::{Decision, RunIo};
use beanagent_core::store::MemoryStore;
use beanagent_core::{RunTurnArgs, run_turn};
use beanagent_llm::{ChatRequest, FakeProvider, LlmError, LlmProvider};
use beanagent_security::CapWorkspace;
use beanagent_skills::{SkillCatalog, skill_tools};
use beanagent_tools::ToolRegistry;
use beanagent_types::{Config, LlmResponse, Role, RolePermissions, SessionId, ToolCall};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
struct RequestSnapshot {
    system: String,
    tools: Vec<String>,
    messages: Vec<(Role, Option<String>)>,
}

#[derive(Debug)]
struct InspectingProvider {
    inner: FakeProvider,
    seen: Arc<Mutex<Vec<RequestSnapshot>>>,
}

#[async_trait]
impl LlmProvider for InspectingProvider {
    async fn chat(&self, request: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        self.seen.lock().unwrap().push(RequestSnapshot {
            system: request.system.to_string(),
            tools: request.tools.iter().map(|tool| tool.name.clone()).collect(),
            messages: request
                .messages
                .iter()
                .map(|message| (message.role, message.text.clone()))
                .collect(),
        });
        self.inner.chat(request).await
    }

    fn name(&self) -> &'static str {
        "inspecting-fake"
    }
}

struct NoopIo {
    cancel: CancellationToken,
}

#[async_trait]
impl RunIo for NoopIo {
    fn on_text(&self, _text: &str) {}
    fn on_tool_start(
        &self,
        _id: &str,
        _tool: &str,
        _risk: beanagent_types::Risk,
        _summary: &str,
        _args: &str,
    ) {
    }
    fn on_tool_end(&self, _id: &str, _tool: &str, _ok: bool, _output: &str) {}

    async fn confirm(
        &self,
        _id: &str,
        _tool: &str,
        _risk: beanagent_types::Risk,
        _prompt: &str,
        _allow_in_session: bool,
        _timeout: Duration,
    ) -> Option<Decision> {
        None
    }

    fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}

#[tokio::test]
async fn fake_provider_loads_matching_skill_before_answering() {
    const BODY_MARKER: &str = "PRIVATE_SKILL_BODY_MARKER";

    let temp = tempfile::tempdir().unwrap();
    let skills_root = temp.path().join("skills");
    let skill_dir = skills_root.join("web-research");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        format!(
            "---\nname: web-research\ndescription: Dùng khi cần nghiên cứu chủ đề.\n---\n# Research\n{BODY_MARKER}"
        ),
    )
    .unwrap();

    let catalog =
        SkillCatalog::load_with_create_root(&[skills_root], temp.path().join("user-skills"));
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace);
    for tool in skill_tools(catalog.clone()) {
        registry.register(tool).unwrap();
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = InspectingProvider {
        inner: FakeProvider::new(vec![
            LlmResponse::with_tool_calls(vec![ToolCall::new(
                "load-1",
                "load_skill",
                serde_json::json!({"name": "web-research"}),
            )]),
            LlmResponse::text_only("Đã kiểm tra nguồn theo checklist của skill."),
        ]),
        seen: seen.clone(),
    };
    let io = NoopIo {
        cancel: CancellationToken::new(),
    };
    let store = MemoryStore::new();
    let config = Config::default();
    let skills_index = catalog.index();
    let owned_io = Arc::new(io);
    let perms = RolePermissions::unrestricted("test");

    let output = run_turn(RunTurnArgs {
        store: &store,
        registry: &registry,
        llm: &provider,
        config: &config,
        session: SessionId::new(1),
        user_text: "Hãy nghiên cứu chủ đề này.".to_string(),
        io: owned_io.clone(),
        cancel: owned_io.cancel.clone(),
        session_policy: None,
        audit: None,
        channel: "test",
        skills_index: &skills_index,
        permissions: &perms,
        project: "default",
    })
    .await
    .unwrap();

    assert_eq!(output, "Đã kiểm tra nguồn theo checklist của skill.");
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].system.contains("web-research:"));
    assert!(!requests[0].system.contains(BODY_MARKER));
    assert!(requests[0].tools.iter().any(|name| name == "load_skill"));
    assert!(requests[1].messages.iter().any(|(role, text)| {
        *role == Role::Tool
            && text
                .as_deref()
                .is_some_and(|text| text.contains(BODY_MARKER))
    }));
}
