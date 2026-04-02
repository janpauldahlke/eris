pub mod push;
pub mod list;
pub mod complete;
pub mod remove;

pub use push::AgendaPushTool;
pub use list::AgendaListTool;
pub use complete::AgendaCompleteTool;
pub use remove::AgendaRemoveTool;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct AgendaTask {
    pub id: String,
    pub created_at: u64,
    pub description: String,
    pub status: String,
}
