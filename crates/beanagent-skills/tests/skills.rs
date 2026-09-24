//! Test M6: loader, skill tools và progressive disclosure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use beanagent_security::CapWorkspace;
use beanagent_skills::{SkillCatalog, SkillError, skill_tools};
use beanagent_tools::ToolCtx;
use beanagent_types::{Risk, SessionId};
use tokio_util::sync::CancellationToken;

fn write_skill(root: &Path, directory: &str, content: &str) {
    let dir = root.join(directory);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), content).unwrap();
}

fn tool_context(workspace: &Path) -> ToolCtx {
    ToolCtx {
        workspace: Arc::new(CapWorkspace::open(workspace.to_path_buf()).unwrap()),
        session: SessionId::new(1),
        cancel: CancellationToken::new(),
        untrusted_seen: Arc::new(AtomicBool::new(false)),
    }
}

#[test]
fn loader_accepts_valid_skips_invalid_and_merges_roots() {
    let temp = tempfile::tempdir().unwrap();
    let bundled = temp.path().join("bundled");
    let user = temp.path().join("user");

    write_skill(
        &bundled,
        "daily-briefing",
        "---\nname: daily-briefing\ndescription: Dùng cho bản tóm tắt hằng ngày.\n---\n# Guide\nprivate body",
    );
    write_skill(
        &user,
        "daily-briefing",
        "---\nname: daily-briefing\ndescription: Phiên bản người dùng được ưu tiên.\n---\n# User guide",
    );
    write_skill(
        &bundled,
        "bad_name",
        "---\nname: bad_name\ndescription: Sai kebab-case.\n---\n# Invalid",
    );
    write_skill(
        &bundled,
        "wrong-name",
        "---\nname: another-name\ndescription: Không khớp thư mục.\n---\n# Invalid",
    );
    write_skill(
        &bundled,
        "too-long",
        &format!(
            "---\nname: too-long\ndescription: {}\n---\n# Invalid",
            "x".repeat(301)
        ),
    );
    write_skill(
        &bundled,
        "missing-description",
        "---\nname: missing-description\n---\n# Invalid",
    );
    write_skill(
        &bundled,
        "unknown-field",
        "---\nname: unknown-field\ndescription: Mô tả.\nversion: 1\n---\n# Invalid",
    );

    let catalog = SkillCatalog::load_with_create_root(&[bundled, user.clone()], user);
    let index = catalog.index();
    assert!(index.contains("daily-briefing: Phiên bản người dùng được ưu tiên."));
    assert!(!index.contains("private body"));
    assert_eq!(
        index.lines().count(),
        1,
        "skill lỗi phải bị bỏ qua: {index}"
    );

    let loaded = catalog.get("daily-briefing").unwrap();
    assert_eq!(loaded.description, "Phiên bản người dùng được ưu tiên.");
    assert!(loaded.content.contains("# User guide"));
    assert!(loaded.directory.is_absolute());
    assert!(matches!(
        catalog.get("bad_name"),
        Err(SkillError::InvalidName(_))
    ));
}

#[test]
fn create_skill_is_non_overwriting_and_rejects_path_names() {
    let temp = tempfile::tempdir().unwrap();
    let create_root = temp.path().join("user-skills");
    let catalog = SkillCatalog::load_with_create_root(&[], create_root);

    let created = catalog
        .create(
            "release-checklist",
            "Dùng trước khi phát hành.",
            "# Steps\n1. Test",
        )
        .unwrap();
    assert!(created.directory.join("SKILL.md").is_file());
    assert!(catalog.index().contains("release-checklist:"));

    assert!(matches!(
        catalog.create("release-checklist", "Mô tả.", "# Another"),
        Err(SkillError::AlreadyExists(_))
    ));
    for name in [
        "../escape",
        "folder/skill",
        r"folder\\skill",
        "Uppercase",
        "-leading",
    ] {
        assert!(matches!(
            catalog.create(name, "Mô tả.", "# Body"),
            Err(SkillError::InvalidName(_))
        ));
    }
    assert!(matches!(
        catalog.create("valid-name", &"x".repeat(301), "# Body"),
        Err(SkillError::InvalidDescription(_))
    ));
}

#[tokio::test]
async fn load_skill_is_safe_and_create_skill_is_confirm() {
    let temp = tempfile::tempdir().unwrap();
    let skills_root = temp.path().join("skills");
    write_skill(
        &skills_root,
        "web-research",
        "---\nname: web-research\ndescription: Dùng để nghiên cứu có nguồn.\n---\n# Research\n- Verify every claim.",
    );
    let catalog =
        SkillCatalog::load_with_create_root(&[skills_root], temp.path().join("user-skills"));
    let tools = skill_tools(catalog.clone());
    let names: Vec<String> = tools.iter().map(|tool| tool.spec().name).collect();
    assert_eq!(names, vec!["load_skill", "create_skill"]);

    let load = tools
        .iter()
        .find(|tool| tool.spec().name == "load_skill")
        .unwrap();
    let create = tools
        .iter()
        .find(|tool| tool.spec().name == "create_skill")
        .unwrap();
    assert_eq!(
        load.risk(&serde_json::json!({"name": "web-research"})),
        Risk::Safe
    );
    assert_eq!(create.risk(&serde_json::json!({})), Risk::Confirm);
    assert!(!load.spec().description.is_empty());
    assert!(!create.spec().description.is_empty());
    assert_eq!(load.spec().parameters["additionalProperties"], false);
    assert_eq!(create.spec().parameters["additionalProperties"], false);

    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let ctx = tool_context(&workspace);
    let output = load
        .call(&ctx, serde_json::json!({"name": "web-research"}))
        .await
        .unwrap();
    assert!(output.contains("Thư mục:"));
    assert!(output.contains("Verify every claim."));

    let created = create
        .call(
            &ctx,
            serde_json::json!({
                "name": "new-skill",
                "description": "Dùng cho checklist mới.",
                "body": "# New guide"
            }),
        )
        .await
        .unwrap();
    assert!(created.contains("new-skill"));
    assert!(catalog.get("new-skill").is_ok());
}
