//! Shared dependencies for web tools in a chat session.

use crate::config::{AppConfig, WebConfig};
use crate::tools::web::fetcher::{MockWebFetcher, WebFetcher};
use crate::tools::web::ledger::WebSessionLedger;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub enum WebFetcherKind {
    Mock(Arc<MockWebFetcher>),
    Browser39 { binary: String },
}

#[derive(Clone)]
pub struct WebToolContext {
    pub vault_root: PathBuf,
    pub web: WebConfig,
    pub web_fetch_user_agent: String,
    pub num_ctx: usize,
    pub vault_read_ratio: f32,
    pub web_fetch_max_bytes: usize,
    pub web_allowlist_override: Option<PathBuf>,
    pub ledger: Arc<Mutex<WebSessionLedger>>,
    pub fetcher: WebFetcherKind,
}

impl WebToolContext {
    pub fn from_config(
        config: &AppConfig,
        vault_root: impl AsRef<Path>,
        ledger: Arc<Mutex<WebSessionLedger>>,
        fetcher: WebFetcherKind,
        effective_web_fetch_max_bytes: usize,
    ) -> Self {
        Self {
            vault_root: vault_root.as_ref().to_path_buf(),
            web: config.web.clone(),
            web_fetch_user_agent: config.web_fetch_user_agent.clone(),
            num_ctx: config.num_ctx,
            vault_read_ratio: config.vault_read_ratio,
            web_fetch_max_bytes: effective_web_fetch_max_bytes,
            web_allowlist_override: config.web_allowlist_path.clone(),
            ledger,
            fetcher,
        }
    }

    pub fn fetcher_for_artifact(&self, artifact_id: &str) -> Arc<dyn WebFetcher> {
        match &self.fetcher {
            WebFetcherKind::Mock(m) => m.clone(),
            WebFetcherKind::Browser39 { binary } => {
                let config_path = self
                    .vault_root
                    .join(".fcp/browser39/config.toml");
                let session_dir = self
                    .vault_root
                    .join(".fcp/browser39/sessions")
                    .join(artifact_id);
                Arc::new(crate::tools::web::fetcher::Browser39Fetcher {
                    binary: binary.clone(),
                    config_path,
                    session_dir,
                })
            }
        }
    }
}
