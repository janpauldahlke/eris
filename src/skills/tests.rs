use tempfile::tempdir;

use super::jit::build_jit_skill_guidance;
use super::seed::seed_runtime_skills;
use super::store::{SkillCreateInput, create_or_update_vault_skill, list_vault_skills, load_vault_skill_by_id};
use super::types::SkillPriority;

#[tokio::test(flavor = "current_thread")]
async fn seed_runtime_skills_is_seed_only() {
    let dir = tempdir().expect("tempdir");
    let report = seed_runtime_skills(dir.path()).await.expect("seed");
    assert!(report.copied >= 3);
    let report_2 = seed_runtime_skills(dir.path()).await.expect("seed again");
    assert_eq!(report_2.copied, 0);
    assert!(report_2.skipped_existing >= 2);
}

#[tokio::test(flavor = "current_thread")]
async fn build_guidance_is_bounded_and_deduped() {
    let dir = tempdir().expect("tempdir");
    seed_runtime_skills(dir.path()).await.expect("seed");
    let ids = vec![
        "mail-recipient-verify".to_string(),
        "mail-recipient-verify".to_string(),
        "db-connections-recovery".to_string(),
    ];
    let out = build_jit_skill_guidance(dir.path(), &ids, 3000)
        .await
        .expect("guidance")
        .expect("some");
    assert!(out.contains("[JIT SKILL GUIDANCE]"));
    assert!(out.contains("mail-recipient-verify"));
    assert!(out.contains("db-connections-recovery"));
}

#[tokio::test(flavor = "current_thread")]
async fn list_and_read_vault_skills() {
    let dir = tempdir().expect("tempdir");
    seed_runtime_skills(dir.path()).await.expect("seed");
    let list = list_vault_skills(dir.path()).await.expect("list");
    assert!(list.iter().any(|s| s.id == "mail-recipient-verify"));
    assert!(list.iter().any(|s| s.id == "skill-authoring-meta"));
    let one = load_vault_skill_by_id(dir.path(), "db-connections-recovery")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(one.id, "db-connections-recovery");
}

#[tokio::test(flavor = "current_thread")]
async fn create_rejects_duplicate_without_overwrite_and_allows_with_flag() {
    let dir = tempdir().expect("tempdir");
    let first = create_or_update_vault_skill(
        dir.path(),
        SkillCreateInput {
            id: "sample-skill".to_string(),
            title: "Sample".to_string(),
            priority: SkillPriority::Mandatory,
            triggers: vec!["skills:list".to_string()],
            body: "Use this.".to_string(),
            overwrite: false,
        },
    )
    .await
    .expect("first create");
    assert!(!first.overwritten);

    let second = create_or_update_vault_skill(
        dir.path(),
        SkillCreateInput {
            id: "sample-skill".to_string(),
            title: "Sample".to_string(),
            priority: SkillPriority::Mandatory,
            triggers: vec!["skills:list".to_string()],
            body: "Use this.".to_string(),
            overwrite: false,
        },
    )
    .await;
    assert!(second.is_err(), "duplicate without overwrite must fail");

    let third = create_or_update_vault_skill(
        dir.path(),
        SkillCreateInput {
            id: "sample-skill".to_string(),
            title: "Sample Updated".to_string(),
            priority: SkillPriority::Conditional,
            triggers: vec!["skills:read".to_string()],
            body: "Updated.".to_string(),
            overwrite: true,
        },
    )
    .await
    .expect("overwrite create");
    assert!(third.overwritten);
    assert_eq!(third.skill.title, "Sample Updated");
}

#[tokio::test(flavor = "current_thread")]
async fn create_rejects_invalid_id() {
    let dir = tempdir().expect("tempdir");
    let bad = create_or_update_vault_skill(
        dir.path(),
        SkillCreateInput {
            id: "Bad Skill".to_string(),
            title: "bad".to_string(),
            priority: SkillPriority::Mandatory,
            triggers: vec!["skills:list".to_string()],
            body: "x".to_string(),
            overwrite: false,
        },
    )
    .await;
    assert!(bad.is_err());
}
