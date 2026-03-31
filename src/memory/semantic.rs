use crate::executive::error::{FcpError, Result};
use crate::config::AppConfig;
use std::sync::Arc;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{CreateCollectionBuilder, Distance, VectorParamsBuilder, PointStruct, SearchPointsBuilder, UpsertPointsBuilder};
use ollama_rs::Ollama;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use std::collections::HashMap;

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

    pub async fn ingest_vault(&self, vault_root: &std::path::Path) -> Result<usize> {
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
                            let (tags, content) = parse_vault_md(&raw);
                            if content.trim().is_empty() {
                                continue;
                            }
                            if let Err(e) = self.upsert(&content, tags).await {
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

    pub async fn search(&self, query: &str, limit: u64) -> Result<String> {
        let embedding = self.generate_embedding(query).await?;

        let search_result = self.client.search_points(
            SearchPointsBuilder::new(&self.config.qdrant_collection, embedding, limit)
                .with_payload(true)
        ).await.map_err(|e| FcpError::NetworkFault(e.to_string()))?;

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

fn parse_vault_md(raw: &str) -> (Vec<String>, String) {
    if !raw.starts_with("---") {
        return (vec![], raw.to_string());
    }

    let after_first = &raw[3..];
    let Some(end) = after_first.find("---") else {
        return (vec![], raw.to_string());
    };

    let frontmatter = &after_first[..end];
    let content = &after_first[end + 3..];

    let mut tags = Vec::new();
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
        if in_tags {
            if let Some(tag) = trimmed.strip_prefix("- ") {
                tags.push(tag.trim().to_string());
            } else {
                in_tags = false;
            }
        }
    }

    (tags, content.trim().to_string())
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
}

