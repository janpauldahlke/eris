pub const MAIL_RECIPIENT_VERIFY_RAW: &str =
    include_str!("defaults/mail-recipient-verify.md");
pub const DB_CONNECTIONS_RECOVERY_RAW: &str =
    include_str!("defaults/db-connections-recovery.md");
pub const SKILL_AUTHORING_META_RAW: &str =
    include_str!("defaults/skill-authoring-meta.md");

#[derive(Debug, Clone)]
pub struct EmbeddedSkill {
    pub file_name: &'static str,
    pub raw: &'static str,
}

pub fn embedded_defaults() -> [EmbeddedSkill; 3] {
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
    ]
}
