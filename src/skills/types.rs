#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillPriority {
    Mandatory,
    Conditional,
}

#[derive(Debug, Clone)]
pub struct SkillDoc {
    pub id: String,
    pub title: String,
    pub priority: SkillPriority,
    pub triggers: Vec<String>,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkillSeedReport {
    pub copied: usize,
    pub skipped_existing: usize,
}
