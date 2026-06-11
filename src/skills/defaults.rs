pub const MAIL_RECIPIENT_VERIFY_RAW: &str =
    include_str!("defaults/mail-recipient-verify.md");
pub const DB_CONNECTIONS_RECOVERY_RAW: &str =
    include_str!("defaults/db-connections-recovery.md");
pub const SKILL_AUTHORING_META_RAW: &str =
    include_str!("defaults/skill-authoring-meta.md");
pub const AGENDA_SELF_LOOP_RAW: &str =
    include_str!("defaults/agenda-self-loop.md");
pub const VAULT_ORIENTATION_RAW: &str =
    include_str!("defaults/vault-orientation.md");
pub const WEB_FETCH_WORKFLOW_RAW: &str =
    include_str!("defaults/web-fetch-workflow.md");
pub const MEDIA_CATALOG_WORKFLOW_RAW: &str =
    include_str!("defaults/media-catalog-workflow.md");

#[derive(Debug, Clone)]
pub struct EmbeddedSkill {
    pub file_name: &'static str,
    pub raw: &'static str,
}

pub fn embedded_defaults() -> [EmbeddedSkill; 7] {
    [
        EmbeddedSkill {
            file_name: "mail-recipient-verify.md",
            raw: MAIL_RECIPIENT_VERIFY_RAW,
        },
        EmbeddedSkill {
            file_name: "db-connections-recovery.md",
            raw: DB_CONNECTIONS_RECOVERY_RAW,
        },
        EmbeddedSkill {
            file_name: "skill-authoring-meta.md",
            raw: SKILL_AUTHORING_META_RAW,
        },
        EmbeddedSkill {
            file_name: "agenda-self-loop.md",
            raw: AGENDA_SELF_LOOP_RAW,
        },
        EmbeddedSkill {
            file_name: "vault-orientation.md",
            raw: VAULT_ORIENTATION_RAW,
        },
        EmbeddedSkill {
            file_name: "web-fetch-workflow.md",
            raw: WEB_FETCH_WORKFLOW_RAW,
        },
        EmbeddedSkill {
            file_name: "media-catalog-workflow.md",
            raw: MEDIA_CATALOG_WORKFLOW_RAW,
        },
    ]
}
