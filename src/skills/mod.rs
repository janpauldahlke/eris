mod defaults;
mod jit;
mod parse;
mod paths;
mod seed;
mod store;
#[cfg(test)]
mod tests;
mod types;

pub use jit::build_jit_skill_guidance;
pub use parse::parse_skill_markdown;
pub use paths::runtime_skills_dir;
pub use seed::seed_runtime_skills;
pub use store::{
    SkillCreateInput, SkillWriteReceipt, create_or_update_vault_skill, list_vault_skills,
    load_vault_skill_by_id,
};
pub use types::{SkillDoc, SkillPriority, SkillSeedReport};
