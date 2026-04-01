use crate::executive::error::{FcpError, Result};
use crate::config::AppConfig;
use crate::memory::ephemeral::is_web_artifact_staging;
use std::sync::Arc;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance, Filter, VectorParamsBuilder, PointStruct, SearchPointsBuilder, UpsertPointsBuilder};
use ollama_rs::Ollama;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use std::collections::HashMap;

/// Parsed vault markdown frontmatter + body (used by boot ingest).
#[derive(Debug, Clone)]
pub struct ParsedVaultMd {
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub content: String,
}

/// Result of [`SemanticBrain::search_memory_query`] (single query embedding; optional Qdrant fallback).
#[derive(Debug, Clone)]
pub struct MemorySearchOutcome {
    pub markdown: String,
    /// True when a filtered search returned zero hits but an unfiltered search with the same vector returned hits.
    pub used_fallback: bool,
    pub attempted_filter_tag: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SemanticChunkHit {
    pub chunk_index: usize,
    pub snippet: String,
    pub score: f32,
}

#[derive(Clone)]
pub struct SemanticBrain {
    client: Arc<Qdrant>,
    ollama: Arc<Ollama>,
    config: Arc<AppConfig>,
}

impl SemanticBrain {
    pub async fn new(config: Arc<AppConfig>, ollama: Arc<Ollama>) -> Result<Self> {
        let client = Qdrant::from_url(&config.qdrant_url)
            .build()
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let collection_name = &config.qdrant_collection;

        let exists = client.collection_exists(collection_name)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        if !exists {
            client.create_collection(
                CreateCollectionBuilder::new(collection_name)
                    .vectors_config(VectorParamsBuilder::new(768, Distance::Cosine))
            ).await.map_err(|e| FcpError::NetworkFault(e.to_string()))?;
        }

        Ok(Self {
            client: Arc::new(client),
            ollama,
            config,
        })
    }

    /// Qdrant gRPC connect with bounded retries. Peripheral checks only TCP; this covers the gap
    /// where the port accepts connections before gRPC is fully ready.
    pub async fn new_with_connect_retries(
        config: Arc<AppConfig>,
        ollama: Arc<Ollama>,
        max_attempts: u32,
        retry_delay_ms: u64,
    ) -> Result<Self> {
        let attempts = max_attempts.max(1);
        let mut last_err: Option<FcpError> = None;

        for attempt in 1..=attempts {
            match Self::new(config.clone(), ollama.clone()).await {
                Ok(brain) => {
                    if attempt > 1 {
                        tracing::info!(
                            attempt,
                            attempts,
                            "Semantic Brain online after gRPC connect retries"
                        );
                    }
                    return Ok(brain);
                }
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        attempts,
                        error = %e,
                        "Semantic Brain gRPC connect attempt failed"
                    );
                    last_err = Some(e);
                    if attempt < attempts {
                        tokio::time::sleep(std::time::Duration::from_millis(retry_delay_ms)).await;
                    }
                }
            }
        }

        match last_err {
            Some(e) => Err(e),
            None => Err(FcpError::NetworkFault(
                "Semantic Brain: connect retries exhausted without error detail".to_string(),
            )),
        }
    }

    pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() {
            return Err(FcpError::EmbeddingFault("Cannot generate embedding for empty query".to_string()));
        }

        let request = GenerateEmbeddingsRequest::new(
            self.config.embed_model_name.clone(),
            text.to_string().into(),
        );

        let response = self.ollama.generate_embeddings(request).await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| FcpError::EmbeddingFault("Embedding model returned no vectors".to_string()))
    }

    pub async fn upsert(&self, text: &str, tags: Vec<String>) -> Result<()> {
        let embedding = self.generate_embedding(text).await?;
        let id = uuid::Uuid::new_v4().to_string();

        let mut payload: HashMap<String, serde_json::Value> = HashMap::new();
        payload.insert("text".to_string(), serde_json::json!(text));
        payload.insert("tags".to_string(), serde_json::json!(tags));

        let point = PointStruct::new(id, embedding, payload);

        self.client.upsert_points(
            UpsertPointsBuilder::new(&self.config.qdrant_collection, vec![point])
        )
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        Ok(())
    }

    /// Upsert a vault file slice with a **stable** point id derived from `vault_relative_key`
    /// (e.g. `40_User/coffee_preference.md`). Reboots overwrite the same point instead of duplicating.
    pub async fn upsert_vault_document(&self, vault_relative_key: &str, text: &str, tags: Vec<String>) -> Result<()> {
        let embedding = self.generate_embedding(text).await?;
        let point_id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, vault_relative_key.as_bytes()).to_string();

        let mut payload: HashMap<String, serde_json::Value> = HashMap::new();
        payload.insert("text".to_string(), serde_json::json!(text));
        payload.insert("tags".to_string(), serde_json::json!(tags));

        let point = PointStruct::new(point_id, embedding, payload);

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.config.qdrant_collection, vec![point]))
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        Ok(())
    }

    pub async fn upsert_web_chunk(
        &self,
        artifact_id: &str,
        url: &str,
        chunk_index: usize,
        text: &str,
    ) -> Result<()> {
        let embedding = self.generate_embedding(text).await?;
        let id = uuid::Uuid::new_v4().to_string();

        let mut payload: HashMap<String, serde_json::Value> = HashMap::new();
        payload.insert("text".to_string(), serde_json::json!(text));
        payload.insert("source".to_string(), serde_json::json!("web_artifact"));
        payload.insert("artifact_id".to_string(), serde_json::json!(artifact_id));
        payload.insert("url".to_string(), serde_json::json!(url));
        payload.insert("chunk_index".to_string(), serde_json::json!(chunk_index));
        payload.insert("tags".to_string(), serde_json::json!(vec!["web_artifact"]));

        let point = PointStruct::new(id, embedding, payload);
        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.config.qdrant_collection, vec![point]))
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        Ok(())
    }

    pub async fn search_web_artifact(
        &self,
        query: &str,
        artifact_id: &str,
        limit: usize,
    ) -> Result<Vec<SemanticChunkHit>> {
        let embedding = self.generate_embedding(query).await?;
        let oversample = (limit.max(1) * 4) as u64;
        let search_result = self
            .client
            .search_points(
                SearchPointsBuilder::new(&self.config.qdrant_collection, embedding, oversample)
                    .with_payload(true),
            )
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let mut out = Vec::new();
        for point in search_result.result {
            let payload = point.payload;
            let Some(artifact_val) = payload.get("artifact_id") else {
                continue;
            };
            let Some(qdrant_client::qdrant::value::Kind::StringValue(found_artifact_id)) = &artifact_val.kind else {
                continue;
            };
            if found_artifact_id != artifact_id {
                continue;
            }

            let text = payload
                .get("text")
                .and_then(|v| v.kind.as_ref())
                .and_then(|k| match k {
                    qdrant_client::qdrant::value::Kind::StringValue(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let chunk_index = payload
                .get("chunk_index")
                .and_then(|v| v.kind.as_ref())
                .and_then(|k| match k {
                    qdrant_client::qdrant::value::Kind::IntegerValue(i) => usize::try_from(*i).ok(),
                    _ => None,
                })
                .unwrap_or(0);

            out.push(SemanticChunkHit {
                chunk_index,
                snippet: text,
                score: point.score,
            });
            if out.len() >= limit.max(1) {
                break;
            }
        }
        Ok(out)
    }

    pub async fn delete_web_artifact_points(&self, artifact_id: &str) -> Result<()> {
        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.config.qdrant_collection)
                    .points(Filter::must([Condition::matches(
                        "artifact_id",
                        artifact_id.to_string(),
                    )]))
                    .wait(true),
            )
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
        Ok(())
    }

    pub async fn ingest_vault(&self, vault_root: &std::path::Path) -> Result<usize> {
        if self.generate_embedding("ping").await.is_err() {
            tracing::warn!(
                "Vault ingest deferred: Ollama unreachable during boot (semantic pre-warm skipped)"
            );
            return Ok(0);
        }

        let subdirs = ["10_Episodic", "20_Semantic", "30_Persons", "40_User"];
        let mut count = 0usize;

        for subdir in &subdirs {
            let dir = vault_root.join(subdir);
            if !dir.exists() {
                continue;
            }

            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(dir = %dir.display(), error = %e, "Failed to read vault subdir");
                    continue;
                }
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md") {
                    match tokio::fs::read_to_string(&path).await {
                        Ok(raw) => {
                            let parsed = parse_vault_md(&raw);
                            let stem = path
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            if is_web_artifact_staging(&parsed.tags, &stem) {
                                tracing::debug!(
                                    path = %path.display(),
                                    "Skipping web artifact markdown from vault ingest"
                                );
                                continue;
                            }
                            if parsed.content.trim().is_empty() {
                                continue;
                            }
                            let file_name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("note.md");
                            let vault_relative_key = format!("{subdir}/{file_name}");
                            let embed_text = vault_embed_text(parsed.title.as_deref(), &parsed.tags, &parsed.content);
                            if let Err(e) = self
                                .upsert_vault_document(&vault_relative_key, &embed_text, parsed.tags)
                                .await
                            {
                                tracing::warn!(path = %path.display(), error = %e, "Failed to ingest vault file");
                            } else {
                                count += 1;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(path = %path.display(), error = %e, "Failed to read vault file");
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    /// One query embedding, then Qdrant search with optional tag filter; if the filter yields no
    /// hits, repeats search with the **same** vector without a filter (no second Ollama call).
    pub async fn search_memory_query(
        &self,
        query: &str,
        limit: u64,
        filter_tag: Option<&str>,
    ) -> Result<MemorySearchOutcome> {
        let embedding = self.generate_embedding(query).await?;
        let trimmed_filter = filter_tag.map(str::trim).filter(|t| !t.is_empty());

        let mut markdown = self
            .search_points_markdown(&embedding, limit, trimmed_filter)
            .await?;
        let mut used_fallback = false;
        let mut attempted_filter_tag: Option<String> = None;

        if let Some(tag) = trimmed_filter {
            if markdown.trim().is_empty() {
                attempted_filter_tag = Some(tag.to_string());
                let unfiltered = self.search_points_markdown(&embedding, limit, None).await?;
                if !unfiltered.trim().is_empty() {
                    markdown = unfiltered;
                    used_fallback = true;
                    tracing::info!(
                        query = %query,
                        attempted_tag = %tag,
                        "memory:query used global search after tag filter returned no hits"
                    );
                }
            }
        }

        Ok(MemorySearchOutcome {
            markdown,
            used_fallback,
            attempted_filter_tag: if used_fallback {
                attempted_filter_tag
            } else {
                None
            },
        })
    }

    async fn search_points_markdown(
        &self,
        embedding: &[f32],
        limit: u64,
        filter_tag: Option<&str>,
    ) -> Result<String> {
        let mut builder =
            SearchPointsBuilder::new(&self.config.qdrant_collection, embedding.to_vec(), limit)
                .with_payload(true);

        if let Some(tag) = filter_tag.map(str::trim).filter(|t| !t.is_empty()) {
            builder = builder.filter(Filter::must([Condition::matches(
                "tags",
                tag.to_string(),
            )]));
        }

        let search_result = self
            .client
            .search_points(builder)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let mut markdown = String::new();
        for point in search_result.result {
            let payload = point.payload;
            if let Some(text_val) = payload.get("text")
                && let Some(qdrant_client::qdrant::value::Kind::StringValue(text)) = &text_val.kind {
                    markdown.push_str(&format!("- {}\n", text));
                }
        }

        Ok(markdown)
    }
}

/// Text stored in Qdrant and embedded for vault-sourced memory (title/tags header + body).
pub fn vault_embed_text(title: Option<&str>, tags: &[String], body: &str) -> String {
    let mut out = String::new();
    if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
        out.push_str("Title: ");
        out.push_str(t);
        out.push('\n');
    }
    if !tags.is_empty() {
        out.push_str("Tags: ");
        out.push_str(&tags.join(", "));
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(body.trim());
    out
}

fn parse_vault_md(raw: &str) -> ParsedVaultMd {
    if !raw.starts_with("---") {
        return ParsedVaultMd {
            title: None,
            tags: Vec::new(),
            content: raw.to_string(),
        };
    }

    let after_first = &raw[3..];
    let Some(end) = after_first.find("---") else {
        return ParsedVaultMd {
            title: None,
            tags: Vec::new(),
            content: raw.to_string(),
        };
    };

    let frontmatter = &after_first[..end];
    let content = &after_first[end + 3..];

    let mut tags = Vec::new();
    let mut title: Option<String> = None;
    let mut in_tags = false;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("tags:") {
            in_tags = true;
            let inline = trimmed.strip_prefix("tags:").unwrap_or("").trim();
            if !inline.is_empty() {
                tags.push(inline.to_string());
            }
            continue;
        }
        if trimmed.starts_with("title:") {
            in_tags = false;
            let v = trimmed.strip_prefix("title:").unwrap_or("").trim();
            let unquoted = v.trim_matches(|c| c == '"' || c == '\'');
            if !unquoted.is_empty() {
                title = Some(unquoted.to_string());
            }
            continue;
        }
        if in_tags {
            if let Some(tag) = trimmed.strip_prefix("- ") {
                tags.push(tag.trim().to_string());
            } else {
                in_tags = false;
            }
        }
    }

    ParsedVaultMd {
        title,
        tags,
        content: content.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use std::sync::Arc;
    use ollama_rs::Ollama;

    #[tokio::test]
    async fn test_semantic_brain_offline_returns_vector_db_offline() {
        let mut config = AppConfig::default();
        config.qdrant_url = "http://localhost:65535".to_string(); // Dead port
        
        let client = Ollama::new("http://localhost".to_string(), 11434);
        let brain_result = SemanticBrain::new(Arc::new(config), Arc::new(client)).await;
        
        match brain_result {
            Err(FcpError::NetworkFault(_)) => (),
            _ => panic!("Expected NetworkFault error, got success instead"),
        }
    }

    #[tokio::test]
    async fn test_semantic_brain_connect_retries_exhaust_dead_port() {
        let mut config = AppConfig::default();
        config.qdrant_url = "http://127.0.0.1:65535".to_string();
        let client = Ollama::new("http://localhost".to_string(), 11434);
        let brain_result = SemanticBrain::new_with_connect_retries(
            Arc::new(config),
            Arc::new(client),
            2,
            1,
        )
        .await;
        assert!(brain_result.is_err(), "expected failure after retries on dead port");
    }

    #[test]
    fn vault_embed_text_includes_title_and_tags() {
        let s = vault_embed_text(
            Some("coffee_preference"),
            &["user".to_string(), "about_me".to_string()],
            "Body line.",
        );
        assert!(s.contains("Title: coffee_preference"));
        assert!(s.contains("Tags: user, about_me"));
        assert!(s.contains("Body line."));
    }

    #[test]
    fn parse_vault_md_extracts_title_and_list_tags() {
        let raw = r#"---
title: "coffee_preference"
tags:
  - user
  - about_me
---

Hello there."#;
        let p = parse_vault_md(raw);
        assert_eq!(p.title.as_deref(), Some("coffee_preference"));
        assert_eq!(p.tags, vec!["user", "about_me"]);
        assert_eq!(p.content, "Hello there.");
    }

    #[test]
    fn vault_relative_key_uuid_stable() {
        let key = "40_User/coffee_preference.md";
        let a = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes());
        let b = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes());
        assert_eq!(a, b);
    }
}

