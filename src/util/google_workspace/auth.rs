use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::executive::error::{FcpError, Result};

const TOKEN_LIFETIME_SECS: u64 = 3600;
const REFRESH_BUFFER_SECS: u64 = 100;
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

#[derive(Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: Option<String>,
}

#[derive(Serialize)]
struct JwtClaims {
    iss: String,
    sub: String,
    scope: String,
    aud: String,
    iat: u64,
    exp: u64,
}

struct CachedToken {
    access_token: String,
    expires_at: u64,
}

pub struct GoogleAuth {
    client_email: String,
    encoding_key: EncodingKey,
    token_uri: String,
    subject: String,
    scopes: String,
    http: reqwest::Client,
    cache: Arc<RwLock<Option<CachedToken>>>,
}

impl GoogleAuth {
    pub async fn from_service_account_key(
        key_path: &Path,
        impersonate_user: &str,
        scopes: &[&str],
    ) -> Result<Self> {
        let raw = tokio::fs::read_to_string(key_path).await.map_err(|e| {
            FcpError::Config(format!(
                "cannot read service account key at {}: {e}",
                key_path.display()
            ))
        })?;
        let sa: ServiceAccountKey = serde_json::from_str(&raw).map_err(|e| {
            FcpError::Config(format!("invalid service account JSON: {e}"))
        })?;
        let encoding_key = EncodingKey::from_rsa_pem(sa.private_key.as_bytes()).map_err(|e| {
            FcpError::Config(format!("invalid RSA private key in service account JSON: {e}"))
        })?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| FcpError::NetworkFault(format!("http client init: {e}")))?;

        Ok(Self {
            client_email: sa.client_email,
            encoding_key,
            token_uri: sa.token_uri.unwrap_or_else(|| TOKEN_URI.to_string()),
            subject: impersonate_user.to_string(),
            scopes: scopes.join(" "),
            http,
            cache: Arc::new(RwLock::new(None)),
        })
    }

    pub async fn access_token(&self) -> Result<String> {
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                let now = now_epoch_secs();
                if now < cached.expires_at.saturating_sub(REFRESH_BUFFER_SECS) {
                    return Ok(cached.access_token.clone());
                }
            }
        }
        let token = self.fetch_token().await?;
        let mut guard = self.cache.write().await;
        *guard = Some(CachedToken {
            access_token: token.clone(),
            expires_at: now_epoch_secs() + TOKEN_LIFETIME_SECS,
        });
        Ok(token)
    }

    async fn fetch_token(&self) -> Result<String> {
        let now = now_epoch_secs();
        let claims = JwtClaims {
            iss: self.client_email.clone(),
            sub: self.subject.clone(),
            scope: self.scopes.clone(),
            aud: self.token_uri.clone(),
            iat: now,
            exp: now + TOKEN_LIFETIME_SECS,
        };
        let header = Header::new(Algorithm::RS256);
        let assertion = jsonwebtoken::encode(&header, &claims, &self.encoding_key)
            .map_err(|e| FcpError::NetworkFault(format!("JWT signing failed: {e}")))?;

        tracing::debug!(
            client_email = %self.client_email,
            subject = %self.subject,
            "requesting Google OAuth2 access token"
        );

        let resp = self
            .http
            .post(&self.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &assertion),
            ])
            .send()
            .await
            .map_err(|e| FcpError::NetworkFault(format!("token endpoint unreachable: {e}")))?;

        let status = resp.status();
        let body = resp.text().await.map_err(|e| {
            FcpError::NetworkFault(format!("token response body read failed: {e}"))
        })?;

        if !status.is_success() {
            tracing::error!(status = %status, body = %body, "Google OAuth2 token request failed");
            return Err(FcpError::NetworkFault(format!(
                "Google OAuth2 token request failed (HTTP {status})"
            )));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }
        let parsed: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            FcpError::NetworkFault(format!("unexpected token response JSON: {e}"))
        })?;

        tracing::info!("Google OAuth2 access token acquired");
        Ok(parsed.access_token)
    }
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_claims_serializes_correctly() {
        let claims = JwtClaims {
            iss: "test@sa.iam.gserviceaccount.com".into(),
            sub: "user@example.com".into(),
            scope: "https://mail.google.com/ https://www.googleapis.com/auth/calendar".into(),
            aud: TOKEN_URI.into(),
            iat: 1000,
            exp: 4600,
        };
        let json = serde_json::to_value(&claims).expect("serialize");
        assert_eq!(json["iss"], "test@sa.iam.gserviceaccount.com");
        assert_eq!(json["sub"], "user@example.com");
        assert_eq!(json["iat"], 1000);
        assert_eq!(json["exp"], 4600);
    }

    #[test]
    fn cached_token_expires_correctly() {
        let now = now_epoch_secs();
        let cached = CachedToken {
            access_token: "tok".into(),
            expires_at: now + 3600,
        };
        assert!(now < cached.expires_at.saturating_sub(REFRESH_BUFFER_SECS));

        let expired = CachedToken {
            access_token: "tok".into(),
            expires_at: now + 50,
        };
        assert!(!(now < expired.expires_at.saturating_sub(REFRESH_BUFFER_SECS)));
    }
}
