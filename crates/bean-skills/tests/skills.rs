//! Test M6: loader, skill tools và progressive disclosure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bean_security::CapWorkspace;
use bean_skills::{
    NewSkillDraft, SkillCatalog, SkillDraftKind, SkillDraftStatus, SkillError, skill_tools,
};
use bean_tools::ToolCtx;
use bean_types::{Risk, SessionId};
use tokio_util::sync::CancellationToken;

fn write_skill(root: &Path, directory: &str, content: &str) {
    let dir = root.join(directory);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), content).unwrap();
}

fn tool_context(workspace: &Path) -> ToolCtx {
    ToolCtx::for_project(
        Arc::new(CapWorkspace::open(workspace.to_path_buf()).unwrap()),
        SessionId::new(1),
        CancellationToken::new(),
        Arc::new(AtomicBool::new(false)),
    )
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

#[test]
fn bundled_skills_in_repository_all_parse() {
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
    let directories: Vec<String> = fs::read_dir(&bundled)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();
            entry
                .file_type()
                .unwrap()
                .is_dir()
                .then(|| entry.file_name().to_string_lossy().into_owned())
        })
        .filter(|name| name != "_drafts")
        .collect();
    assert!(
        !directories.is_empty(),
        "repo phải có ít nhất một skill bundled"
    );

    // Loader bỏ qua skill lỗi một cách im lặng nên phải so với danh sách thư mục thật.
    let catalog = SkillCatalog::load(std::slice::from_ref(&bundled));
    for name in &directories {
        let loaded = catalog.get(name);
        assert!(
            loaded.is_ok(),
            "skill bundled `{name}` không nạp được: {:?}",
            loaded.err()
        );
        let skill = loaded.unwrap();
        assert_eq!(skill.name, *name, "tên skill phải khớp thư mục");
        assert!(!skill.description.is_empty());
    }
}

#[test]
fn draft_stays_inactive_until_approve_and_reload() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("skills");
    let catalog = SkillCatalog::load_with_paths(
        std::slice::from_ref(&root),
        root.clone(),
        root.join("_drafts"),
    );
    let draft = catalog
        .create_draft(NewSkillDraft {
            name: "release-checklist".into(),
            kind: SkillDraftKind::New,
            description: "Dùng trước khi phát hành.".into(),
            body: "# Steps\n1. Chạy test.".into(),
            reason: "Một quy trình lặp lại nhiều lần.".into(),
            source_session_id: 7,
            source_channel: "test".into(),
            source_chat_id: "chat".into(),
            created_at: "2026-09-25T10:00:00Z".into(),
        })
        .unwrap();
    assert!(root.join("_drafts/release-checklist/SKILL.md").is_file());
    assert!(catalog.get("release-checklist").is_err());
    assert_eq!(draft.status, SkillDraftStatus::Pending);
    assert!(catalog.last_proposal_at().unwrap().is_some());
    let reloaded = SkillCatalog::load_with_paths(
        std::slice::from_ref(&root),
        root.clone(),
        root.join("_drafts"),
    );
    assert!(reloaded.last_proposal_at().unwrap().is_some());

    let decision = catalog.approve_draft(&draft.id).unwrap();
    assert_eq!(decision.status, SkillDraftStatus::Approved);
    assert!(root.join("release-checklist/SKILL.md").is_file());
    assert!(!root.join("_drafts/release-checklist").exists());
    assert!(catalog.get("release-checklist").is_ok());
}

#[test]
fn malformed_draft_is_skipped_and_invalid_body_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("skills");
    let catalog = SkillCatalog::load_with_paths(
        std::slice::from_ref(&root),
        root.clone(),
        root.join("_drafts"),
    );
    let error = catalog
        .create_draft(NewSkillDraft {
            name: "invalid-skill".into(),
            kind: SkillDraftKind::New,
            description: "Mô tả hợp lệ.".into(),
            body: "   ".into(),
            reason: "Lý do hợp lệ.".into(),
            source_session_id: 1,
            source_channel: "test".into(),
            source_chat_id: "chat".into(),
            created_at: "2026-09-25T10:00:00Z".into(),
        })
        .unwrap_err();
    assert!(matches!(error, SkillError::MissingBody));

    let malformed = root.join("_drafts/malformed");
    std::fs::create_dir_all(&malformed).unwrap();
    std::fs::write(malformed.join("SKILL.md"), "không có frontmatter").unwrap();
    assert!(catalog.list_drafts().unwrap().is_empty());
}

/// (K16 phương án (b)) Đề xuất chờ nằm trong state file: thêm → xem → lấy theo
/// phiên (FIFO, chỉ phiên hỏi) → state rỗng; đề xuất của phiên khác giữ nguyên.
#[test]
fn pending_proposal_roundtrip_is_session_scoped_and_fifo() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("skills");
    let catalog = SkillCatalog::load_with_paths(
        std::slice::from_ref(&root),
        root.clone(),
        root.join("_drafts"),
    );
    let make = |name: &str, session: i64| NewSkillDraft {
        name: name.into(),
        kind: SkillDraftKind::New,
        description: "Mô tả hợp lệ.".into(),
        body: "# Guide".into(),
        reason: "Quy trình có thể tái sử dụng.".into(),
        source_session_id: session,
        source_channel: "test".into(),
        source_chat_id: "chat".into(),
        created_at: "2026-11-13T09:00:00Z".into(),
    };

    assert!(catalog.pending_proposals().unwrap().is_empty());
    catalog
        .add_pending_proposal(make("first-skill", 1))
        .unwrap();
    catalog
        .add_pending_proposal(make("second-skill", 1))
        .unwrap();
    catalog
        .add_pending_proposal(make("other-session", 2))
        .unwrap();

    // Cooldown tick tại thời điểm reflection, không phải lúc thăng hạng.
    assert_eq!(
        catalog.last_proposal_at().unwrap().as_deref(),
        Some("2026-11-13T09:00:00Z")
    );

    let taken = catalog.take_pending_proposals(1).unwrap();
    let names: Vec<&str> = taken.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["first-skill", "second-skill"],
        "phải giữ thứ tự FIFO"
    );
    let pending = catalog.pending_proposals().unwrap();
    let rest: Vec<&str> = pending.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(rest, vec!["other-session"]);

    assert_eq!(catalog.discard_pending_proposals(2).unwrap(), 1);
    assert!(catalog.pending_proposals().unwrap().is_empty());
    assert_eq!(catalog.discard_pending_proposals(2).unwrap(), 0);
}

/// (K16) Đề xuất chờ phải bền qua restart: catalog mới dựng trên cùng thư mục đọc
/// lại được candidate từ `.state.json`.
#[test]
fn pending_proposal_survives_catalog_reload() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("skills");
    let catalog = SkillCatalog::load_with_paths(
        std::slice::from_ref(&root),
        root.clone(),
        root.join("_drafts"),
    );
    catalog
        .add_pending_proposal(NewSkillDraft {
            name: "restart-safe".into(),
            kind: SkillDraftKind::New,
            description: "Mô tả hợp lệ.".into(),
            body: "# Guide".into(),
            reason: "Quy trình có thể tái sử dụng.".into(),
            source_session_id: 3,
            source_channel: "test".into(),
            source_chat_id: "chat".into(),
            created_at: "2026-11-13T09:30:00Z".into(),
        })
        .unwrap();

    let reloaded = SkillCatalog::load_with_paths(
        std::slice::from_ref(&root),
        root.clone(),
        root.join("_drafts"),
    );
    let pending = reloaded.pending_proposals().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "restart-safe");
    assert_eq!(pending[0].source_session_id, 3);
}

/// (K16) Take/discard khi state rỗng không được ghi file: `take_pending_proposals`
/// chạy ở đầu mọi lượt, nếu ghi vô ích thì mỗi tiến trình bean (chat/serve) tạo
/// `.state.json` trong CWD dù learning không bao giờ sinh đề xuất (lỗi thật:
/// `make check` làm bẩn `crates/bean/skills/`).
#[test]
fn take_and_discard_on_empty_state_do_not_create_state_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("skills");
    let catalog = SkillCatalog::load_with_paths(
        std::slice::from_ref(&root),
        root.clone(),
        root.join("_drafts"),
    );
    assert!(catalog.take_pending_proposals(1).unwrap().is_empty());
    assert_eq!(catalog.discard_pending_proposals(1).unwrap(), 0);
    assert!(
        !root.join("_drafts/.state.json").exists(),
        "state rỗng không được ghi ra đĩa"
    );
}

/// (K16) Validate rẻ chạy ngay lúc lưu candidate: tên xấu bị chặn trước khi đụng
/// state; draft phải tồn tại sau `take` + `create_draft` với đề xuất hợp lệ.
#[test]
fn add_pending_proposal_validates_cheaply_and_promotes_via_create_draft() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("skills");
    let catalog = SkillCatalog::load_with_paths(
        std::slice::from_ref(&root),
        root.clone(),
        root.join("_drafts"),
    );
    let mut invalid = NewSkillDraft {
        name: "Not_Kebab".into(),
        kind: SkillDraftKind::New,
        description: "Mô tả.".into(),
        body: "# Guide".into(),
        reason: "Lý do.".into(),
        source_session_id: 1,
        source_channel: "test".into(),
        source_chat_id: "chat".into(),
        created_at: "2026-11-13T09:00:00Z".into(),
    };
    assert!(matches!(
        catalog.add_pending_proposal(invalid.clone()),
        Err(SkillError::InvalidName(_))
    ));
    invalid.name = "kebab-name".into();
    invalid.reason = String::new();
    assert!(catalog.add_pending_proposal(invalid).is_err());
    assert!(catalog.pending_proposals().unwrap().is_empty());

    let valid = NewSkillDraft {
        name: "promoted-later".into(),
        kind: SkillDraftKind::New,
        description: "Mô tả hợp lệ.".into(),
        body: "# Guide".into(),
        reason: "Quy trình có thể tái sử dụng.".into(),
        source_session_id: 1,
        source_channel: "test".into(),
        source_chat_id: "chat".into(),
        created_at: "2026-11-13T09:00:00Z".into(),
    };
    catalog.add_pending_proposal(valid.clone()).unwrap();
    let mut taken = catalog.take_pending_proposals(1).unwrap();
    assert_eq!(taken.len(), 1);
    let draft = catalog.create_draft(taken.remove(0)).unwrap();
    assert_eq!(draft.name, "promoted-later");
    assert!(catalog.list_drafts().unwrap().len() == 1);
}
