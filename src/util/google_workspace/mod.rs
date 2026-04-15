//! Google Workspace API clients (service account + domain-wide delegation).

use std::sync::Arc;

use crate::config::GoogleConfig;
use crate::executive::error::{FcpError, Result};

mod auth;
mod calendar;
mod gmail;

pub use auth::GoogleAuth;
pub use calendar::CalendarClient;
pub use gmail::GmailClient;

/// OAuth scopes on the service-account JWT. Must be a subset of Admin Console → Security → API controls → Domain-wide delegation for this client ID.
pub const GOOGLE_WORKSPACE_API_SCOPES: &[&str] = &[
    "https://mail.google.com/",
    "https://www.googleapis.com/auth/calendar",
];

/// Shared auth for Gmail + Calendar when `[google]` is enabled and credentials are complete.
pub async fn workspace_auth(config: &GoogleConfig) -> Result<Option<Arc<GoogleAuth>>> {
    if !config.enabled {
        tracing::info!("Google integration disabled in config");
        return Ok(None);
    }
    let key_path = config.service_account_key.as_ref().ok_or_else(|| {
        FcpError::Config("google.enabled=true but service_account_key is missing".into())
    })?;
    let user = config.impersonate_user.as_deref().ok_or_else(|| {
        FcpError::Config("google.enabled=true but impersonate_user is missing".into())
    })?;
    let auth = GoogleAuth::from_service_account_key(
        key_path.as_path(),
        user,
        GOOGLE_WORKSPACE_API_SCOPES,
    )
    .await?;
    Ok(Some(Arc::new(auth)))
}
