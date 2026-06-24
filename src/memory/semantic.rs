use crate::config::AppConfig;
use crate::engine::EmbeddingProvider;
use crate::executive::error::{FcpError, Result};
use crate::ingest::truncate_char_boundary;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, DeletePointsBuilder,
    Direction, Distance, FieldType, Filter, GetCollectionInfoResponse, OrderBy, PointStruct,
    ScrollPointsBuilder, SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
    vectors_config::Config as VectorSchemaConfig,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Qdrant payload key for millisecond UNIX time used to order “where we left off” recall.
pub const RECENCY_TS_PAYLOAD_KEY: &str = "recency_ts";

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
    /// True when [`MemoryQueryOptions::vault_path_prefix`] matched no points; a broader search without prefix was used.
    pub used_vault_prefix_fallback: bool,
    pub attempted_vault_prefix: Option<String>,
}

/// Semantic similarity vs. latest persisted memory (by [`RECENCY_TS_PAYLOAD_KEY`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemoryQuerySort {
    #[default]
    Semantic,
    Recency,
}

/// Tunable knobs for [`SemanticBrain::search_memory_query`].
#[derive(Debug, Clone, Copy)]
pub struct MemoryQueryOptions<'a> {
    /// Max points after ranking (clamped by the tool using config).
    pub top_k: u64,
    pub filter_tag: Option<&'a str>,
    /// Only include points whose `vault_key` payload starts with this prefix (e.g. `30_Synthesis/`).
    pub vault_path_prefix: Option<&'a str>,
    pub min_score: Option<f32>,
    pub max_total_chars: usize,
    /// From [`crate::config::AppConfig::memory_query_oversample_cap`].
    pub qdrant_oversample_cap: u64,
    pub qdrant_oversample_multiplier: u64,
    pub qdrant_oversample_min: u64,
    /// [`MemoryQuerySort::Semantic`] uses vector similarity; [`MemoryQuerySort::Recency`] scrolls by `recency_ts` (no embedding call).
    pub memory_sort: MemoryQuerySort,
}

/// Zettelkasten vault directories for v2 ingest.
pub(crate) const VAULT_INGEST_SUBDIRS_V2: &[&str] = &[
    "00_Invariants",
    "10_Topology",
    "20_Discourse",
    "30_Synthesis",
    "40_MEDIA",
];

/// Boot/watch ingest roots for the current config. `40_MEDIA` is included when
/// `[vision] enabled` (images) or `[document_rag] enabled` (document discovery cards).
pub fn vault_ingest_subdirs_for_config(config: &AppConfig) -> Vec<&'static str> {
    VAULT_INGEST_SUBDIRS_V2
        .iter()
        .copied()
        .filter(|subdir| {
            *subdir != "40_MEDIA" || config.vision.enabled || config.document_rag.enabled
        })
        .collect()
}

/// Extract dense vector size from Qdrant collection info (named or single vector).
#[must_use]
pub fn vector_dim_from_collection_info(info: &GetCollectionInfoResponse) -> Option<usize> {
    let ci = info.result.as_ref()?;
    let cfg = ci.config.as_ref()?;
    let params = cfg.params.as_ref()?;
    let vc = params.vectors_config.as_ref()?;
    match &vc.config {
        Some(VectorSchemaConfig::Params(p)) => Some(p.size as usize),
        Some(VectorSchemaConfig::ParamsMap(m)) => m.map.values().next().map(|p| p.size as usize),
        None => None,
    }
}

/// When the collection exists, returns its configured vector dimension; otherwise `None`.
pub async fn collection_vector_dimensions(config: &AppConfig) -> Result<Option<usize>> {
    let client = Qdrant::from_url(&config.qdrant_url)
        .build()
        .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
    let name = config.qdrant_collection_v2.as_str();
    let exists = client
        .collection_exists(name)
        .await
        .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
    if !exists {
        return Ok(None);
    }
    let info = client
        .collection_info(name)
        .await
        .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
    Ok(vector_dim_from_collection_info(&info))
}

/// Fails fast when an existing Qdrant collection dimension disagrees with the embedding provider.
pub(crate) fn embedding_dims_agree_or_config_err(
    embed_dims: usize,
    coll_dims: Option<usize>,
    collection_name: &str,
) -> Result<()> {
    match coll_dims {
        None => Ok(()),
        Some(d) if d == embed_dims => Ok(()),
        Some(d) => Err(FcpError::Config(format!(
            "Embedding dimension mismatch: provider produces {embed_dims}-dim vectors, \
             but Qdrant collection '{collection_name}' is configured for {d}-dim. \
             Either use a compatible embedding model or recreate the collection.",
        ))),
    }
}

/// Compares embedding width with an existing Qdrant collection (after gRPC lookup).
pub async fn validate_embedding_provider_vs_qdrant(
    config: &AppConfig,
    embed_dims: usize,
) -> Result<()> {
    embedding_dims_agree_or_config_err(
        embed_dims,
        collection_vector_dimensions(config).await?,
        config.qdrant_collection_v2.as_str(),
    )
}

/// One vector hit before formatting for the LLM.
#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub score: f32,
    pub text: String,
    pub vault_key: Option<String>,
    /// When set, output uses a recency header instead of a cosine score (see [`format_memory_hits_markdown`]).
    pub recency_ts_ms: Option<u64>,
}

#[derive(Clone)]
pub struct SemanticBrain {
    client: Arc<Qdrant>,
    embed: Arc<dyn EmbeddingProvider>,
    config: Arc<AppConfig>,
}

impl SemanticBrain {
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub async fn new(config: Arc<AppConfig>, embed: Arc<dyn EmbeddingProvider>) -> Result<Self> {
        let client = Qdrant::from_url(&config.qdrant_url)
            .build()
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let collection_name = &config.qdrant_collection_v2;

        let exists = client
            .collection_exists(collection_name)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        if !exists {
            client
                .create_collection(
                    CreateCollectionBuilder::new(collection_name)
                        .vectors_config(VectorParamsBuilder::new(768, Distance::Cosine)),
                )
                .await
                .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
            tracing::info!(collection = %collection_name, "Created Qdrant collection");
        }

        Ok(Self {
            client: Arc::new(client),
            embed,
            config,
        })
    }

    /// Ensures an integer payload index exists for [`RECENCY_TS_PAYLOAD_KEY`] (idempotent).
    async fn ensure_recency_payload_index(&self) -> Result<()> {
        let collection = &self.config.qdrant_collection_v2;
        if collection.is_empty() {
            return Ok(());
        }
        let builder = CreateFieldIndexCollectionBuilder::new(
            collection.clone(),
            RECENCY_TS_PAYLOAD_KEY.to_string(),
            FieldType::Integer,
        )
        .wait(true);
        match self.client.create_field_index(builder).await {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already exists")
                    || msg.contains("AlreadyExists")
                    || msg.contains("duplicate")
                    || msg.contains("Conflict")
                {
                    Ok(())
                } else {
                    Err(FcpError::NetworkFault(format!(
                        "recency_ts payload index: {msg}"
                    )))
                }
            }
        }
    }

    /// Qdrant gRPC connect with bounded retries. Chat startup still uses this for transient
    /// errors after [`crate::executive::peripherals::qdrant_grpc_ready`] (e.g. load spikes).
    pub async fn new_with_connect_retries(
        config: Arc<AppConfig>,
        embed: Arc<dyn EmbeddingProvider>,
        max_attempts: u32,
        retry_delay_ms: u64,
    ) -> Result<Self> {
        let attempts = max_attempts.max(1);
        let mut last_err: Option<FcpError> = None;

        for attempt in 1..=attempts {
            match Self::new(config.clone(), embed.clone()).await {
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
            return Err(FcpError::EmbeddingFault(
                "Cannot generate embedding for empty query".to_string(),
            ));
        }
        self.embed.embed(text).await
    }

    /// `vault_key` should be a stable path-like id (e.g. `30_Synthesis/<node_id>/r0001.md` or `committed:<uuid>`). When `None`, uses `committed:<point_id>`.
    pub async fn upsert(
        &self,
        text: &str,
        tags: Vec<String>,
        vault_key: Option<String>,
    ) -> Result<()> {
        let embedding = self.generate_embedding(text).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let vk = vault_key.unwrap_or_else(|| format!("committed:{id}"));

        let mut payload: HashMap<String, serde_json::Value> = HashMap::new();
        payload.insert("text".to_string(), serde_json::json!(text));
        payload.insert("tags".to_string(), serde_json::json!(tags));
        payload.insert("vault_key".to_string(), serde_json::json!(vk));
        payload.insert(
            RECENCY_TS_PAYLOAD_KEY.to_string(),
            serde_json::json!(unix_ms_now()),
        );

        let point = PointStruct::new(id, embedding, payload);

        self.client
            .upsert_points(UpsertPointsBuilder::new(
                &self.config.qdrant_collection_v2,
                vec![point],
            ))
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        Ok(())
    }

    /// Remove a vault document point by stable `vault_key` (UUID v5 of key).
    pub async fn delete_vault_document_v2(&self, vault_relative_key: &str) -> Result<()> {
        let collection = &self.config.qdrant_collection_v2;
        if collection.is_empty() {
            return Ok(());
        }
        let point_id =
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, vault_relative_key.as_bytes()).to_string();
        self.client
            .delete_points(
                DeletePointsBuilder::new(collection)
                    .points(vec![point_id])
                    .wait(true),
            )
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
        Ok(())
    }

    /// Semantic vector search returning ranked hits (used by turn-start prefetch).
    pub async fn semantic_search_hits(
        &self,
        query: &str,
        top_k: usize,
        min_score: Option<f32>,
    ) -> Result<Vec<MemoryHit>> {
        let embedding = self.generate_embedding(query).await?;
        let limit = u64::try_from(top_k.max(1)).unwrap_or(u64::MAX);
        let mut hits = self.search_points_hits(&embedding, limit, None).await?;
        hits = filter_hits_min_score(hits, min_score);
        Ok(sort_and_limit_hits(hits, top_k))
    }

    /// Upsert a vault document into the **v2** collection with enriched payload fields.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_vault_document_v2(
        &self,
        vault_relative_key: &str,
        text: &str,
        tags: Vec<String>,
        node_id: Option<&str>,
        rev: Option<u32>,
        is_current: bool,
        epistemic_status: Option<&str>,
        recency_ts_ms: u64,
    ) -> Result<()> {
        let collection = &self.config.qdrant_collection_v2;
        if collection.is_empty() {
            return Ok(());
        }

        let embedding = self.generate_embedding(text).await?;
        let point_id =
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, vault_relative_key.as_bytes())
                .to_string();

        let mut payload: HashMap<String, serde_json::Value> = HashMap::new();
        payload.insert("text".into(), serde_json::json!(text));
        payload.insert("tags".into(), serde_json::json!(tags));
        payload.insert("vault_key".into(), serde_json::json!(vault_relative_key));
        payload.insert("is_current".into(), serde_json::json!(is_current));
        if let Some(nid) = node_id {
            payload.insert("node_id".into(), serde_json::json!(nid));
        }
        if let Some(r) = rev {
            payload.insert("rev".into(), serde_json::json!(r));
        }
        if let Some(es) = epistemic_status {
            payload.insert("epistemic_status".into(), serde_json::json!(es));
        }
        payload.insert(
            RECENCY_TS_PAYLOAD_KEY.into(),
            serde_json::json!(recency_ts_ms),
        );

        let point = PointStruct::new(point_id, embedding, payload);
        self.client
            .upsert_points(UpsertPointsBuilder::new(collection, vec![point]))
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        Ok(())
    }

    /// Sync one vault-relative markdown path into Qdrant (or delete if the file is gone).
    pub async fn sync_vault_path(
        &self,
        vault_root: &std::path::Path,
        rel_path: &str,
    ) -> Result<()> {
        let key = rel_path.replace('\\', "/");
        if !vault_key_is_ingest_eligible(&key) {
            return Ok(());
        }

        if key.starts_with("30_Synthesis/") {
            if let Some(node_id) = synthesis_node_id_from_key(&key) {
                let _ = self.index_synthesis_node(vault_root, &node_id).await?;
            }
            return Ok(());
        }

        let abs = vault_root.join(&key);
        if !abs.is_file() {
            self.delete_vault_document_v2(&key).await?;
            tracing::debug!(vault_key = %key, "semantic reindex: removed deleted vault file");
            return Ok(());
        }

        if self.index_tree_md_file(vault_root, &abs).await? {
            tracing::debug!(vault_key = %key, "semantic reindex: indexed vault file");
            return Ok(());
        }
        if self.index_tree_json_file(vault_root, &abs).await? {
            tracing::debug!(vault_key = %key, "semantic reindex: indexed media json");
        }
        Ok(())
    }

    /// Recursive v2 vault ingest. For `30_Synthesis`, only indexes the **current head**
    /// revision per node_id directory. Other roots are tree-ingested.
    pub async fn ingest_vault_v2(&self, vault_root: &std::path::Path) -> Result<usize> {
        if self.config.qdrant_collection_v2.is_empty() {
            return Ok(0);
        }
        if self.generate_embedding("ping").await.is_err() {
            tracing::warn!("v2 vault ingest deferred: embedding provider unreachable during boot");
            return Ok(0);
        }

        let mut count = 0usize;

        let subdirs = vault_ingest_subdirs_for_config(&self.config);
        if !self.config.vision.enabled && !self.config.document_rag.enabled {
            tracing::debug!(
                "v2 ingest: skipping 40_MEDIA (vision and document_rag disabled)"
            );
        } else if !self.config.vision.enabled {
            tracing::debug!(
                "v2 ingest: 40_MEDIA limited to document cards (vision disabled)"
            );
        }

        for subdir in subdirs {
            let dir = vault_root.join(subdir);
            if !dir.exists() {
                continue;
            }

            if subdir == "30_Synthesis" {
                count += self.ingest_synthesis_recursive(vault_root, &dir).await;
            } else {
                count += self.ingest_tree_v2(&dir, vault_root).await;
            }
        }

        tracing::info!(count, "v2 vault ingest complete");
        Ok(count)
    }

    /// Recursively ingest `.md` files under a v2 root (Invariants, Topology, Discourse).
    async fn ingest_tree_v2(&self, dir: &std::path::Path, vault_root: &std::path::Path) -> usize {
        let mut count = 0usize;
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            let mut entries = match tokio::fs::read_dir(&current).await {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(dir = %current.display(), error = %e, "v2 ingest: failed to read dir");
                    continue;
                }
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if self.index_tree_md_file(vault_root, &path).await.unwrap_or(false) {
                    count += 1;
                } else if self.index_tree_json_file(vault_root, &path).await.unwrap_or(false) {
                    count += 1;
                }
            }
        }
        count
    }

    async fn index_tree_json_file(
        &self,
        vault_root: &std::path::Path,
        path: &std::path::Path,
    ) -> Result<bool> {
        if path.extension().is_none_or(|e| e != "json") {
            return Ok(false);
        }
        let vault_key = match vault_key_from_abs_path(vault_root, path) {
            Some(k) if vault_key_is_ingest_eligible(&k) && k.starts_with("40_MEDIA/") => k,
            _ => return Ok(false),
        };

        let raw = tokio::fs::read_to_string(path).await.map_err(FcpError::Io)?;
        let card = crate::media::parse_media_json(&raw)?;
        if !crate::media::media_card_eligible_for_ingest(&self.config, &card) {
            return Ok(false);
        }
        let embed_text = crate::media::build_embed_text(&card);
        if embed_text.trim().is_empty() {
            return Ok(false);
        }
        let recency_ts_ms = card.updated_at.saturating_mul(1000);
        self.upsert_vault_document_v2(
            &vault_key,
            &embed_text,
            card.tags.clone(),
            None,
            None,
            true,
            None,
            recency_ts_ms,
        )
        .await?;
        tracing::debug!(vault_key = %vault_key, "v2 ingest: indexed media json");
        Ok(true)
    }

    async fn index_tree_md_file(
        &self,
        vault_root: &std::path::Path,
        path: &std::path::Path,
    ) -> Result<bool> {
        if path.extension().is_none_or(|e| e != "md") {
            return Ok(false);
        }
        let normalized = path.to_string_lossy().replace('\\', "/");
        if normalized.contains("/web/missions/") {
            return Ok(false);
        }
        let vault_key = match vault_key_from_abs_path(vault_root, path) {
            Some(k) if vault_key_is_ingest_eligible(&k) => k,
            _ => return Ok(false),
        };

        let raw = tokio::fs::read_to_string(path).await.map_err(FcpError::Io)?;
        let parsed = parse_vault_md(&raw);
        if parsed.content.trim().is_empty() {
            return Ok(false);
        }
        let recency_ts_ms = tokio::fs::metadata(path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(system_time_to_unix_ms)
            .unwrap_or_else(unix_ms_now);
        let embed_text = vault_embed_text(parsed.title.as_deref(), &parsed.tags, &parsed.content);
        let epistemic = extract_frontmatter_field(&raw, "epistemic_status");

        self.upsert_vault_document_v2(
            &vault_key,
            &embed_text,
            parsed.tags,
            None,
            None,
            true,
            epistemic.as_deref(),
            recency_ts_ms,
        )
        .await?;
        tracing::debug!(vault_key = %vault_key, "v2 ingest: indexed file");
        Ok(true)
    }

    async fn index_synthesis_node(&self, vault_root: &std::path::Path, node_id: &str) -> Result<bool> {
        let node_path = vault_root.join("30_Synthesis").join(node_id);
        if !node_path.is_dir() {
            return Ok(false);
        }
        let subdir = "30_Synthesis";
        let mut best_rev: Option<(u32, std::path::PathBuf)> = None;
        let mut rev_entries = match tokio::fs::read_dir(&node_path).await {
            Ok(e) => e,
            Err(_) => return Ok(false),
        };
        while let Ok(Some(rev_entry)) = rev_entries.next_entry().await {
            let name = rev_entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(num_str) = name_str
                .strip_prefix('r')
                .and_then(|s| s.strip_suffix(".md"))
                && let Ok(n) = num_str.parse::<u32>()
                && best_rev.as_ref().is_none_or(|(best, _)| n > *best)
            {
                best_rev = Some((n, rev_entry.path()));
            }
        }
        let Some((rev, head_path)) = best_rev else {
            return Ok(false);
        };

        let raw = tokio::fs::read_to_string(&head_path).await.map_err(FcpError::Io)?;
        let parsed = parse_vault_md(&raw);
        if parsed.content.trim().is_empty() {
            return Ok(false);
        }
        let recency_ts_ms = tokio::fs::metadata(&head_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(system_time_to_unix_ms)
            .unwrap_or_else(unix_ms_now);
        let file_name = head_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("r0001.md");
        let vault_key = format!("{subdir}/{node_id}/{file_name}");
        let embed_text = vault_embed_text(parsed.title.as_deref(), &parsed.tags, &parsed.content);
        let epistemic = extract_frontmatter_field(&raw, "epistemic_status");

        self.upsert_vault_document_v2(
            &vault_key,
            &embed_text,
            parsed.tags,
            Some(node_id),
            Some(rev),
            true,
            epistemic.as_deref(),
            recency_ts_ms,
        )
        .await?;
        Ok(true)
    }

    /// Ingest `30_Synthesis/<node_id>/rXXXX.md` — only the highest-rev head per node.
    async fn ingest_synthesis_recursive(
        &self,
        vault_root: &std::path::Path,
        synth_dir: &std::path::Path,
    ) -> usize {
        let mut count = 0usize;
        let mut node_dirs = match tokio::fs::read_dir(synth_dir).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir = %synth_dir.display(), error = %e, "v2 ingest: failed to read synthesis dir");
                return 0;
            }
        };

        while let Ok(Some(node_entry)) = node_dirs.next_entry().await {
            let node_path = node_entry.path();
            if !node_path.is_dir() {
                continue;
            }
            let node_id = node_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if node_id.is_empty() {
                continue;
            }

            match self.index_synthesis_node(vault_root, &node_id).await {
                Ok(true) => {
                    tracing::debug!(node_id = %node_id, "v2 ingest: indexed synthesis head revision");
                    count += 1;
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(node_id = %node_id, error = %e, "v2 ingest: failed to index synthesis head");
                }
            }
        }
        count
    }

    /// Semantic search uses one query embedding; optional tag filter with global fallback.
    /// [`MemoryQuerySort::Recency`] skips embeddings and scrolls by [`RECENCY_TS_PAYLOAD_KEY`].
    pub async fn search_memory_query(
        &self,
        query: &str,
        options: MemoryQueryOptions<'_>,
    ) -> Result<MemorySearchOutcome> {
        if options.memory_sort == MemoryQuerySort::Recency {
            self.ensure_recency_payload_index().await?;
            return self.search_memory_query_recency(query, options).await;
        }

        let embedding = self.generate_embedding(query).await?;
        let trimmed_tag = options.filter_tag.map(str::trim).filter(|t| !t.is_empty());
        let trimmed_prefix = options
            .vault_path_prefix
            .map(str::trim)
            .filter(|t| !t.is_empty());

        let qdrant_limit = qdrant_oversample_limit(
            options.top_k,
            trimmed_prefix.is_some(),
            options.qdrant_oversample_cap,
            options.qdrant_oversample_multiplier,
            options.qdrant_oversample_min,
        );

        tracing::debug!(
            top_k = options.top_k,
            qdrant_limit,
            has_tag = trimmed_tag.is_some(),
            has_prefix = trimmed_prefix.is_some(),
            min_score = ?options.min_score,
            max_total_chars = options.max_total_chars,
            "memory:query search_memory_query"
        );

        let top_k = (options.top_k as usize).max(1);

        let mut markdown = self
            .run_memory_query_pipeline(
                &embedding,
                qdrant_limit,
                trimmed_tag,
                trimmed_prefix,
                options.min_score,
                top_k,
                options.max_total_chars,
            )
            .await?;

        let mut used_fallback = false;
        let mut attempted_filter_tag: Option<String> = None;
        let mut used_vault_prefix_fallback = false;
        let mut attempted_vault_prefix: Option<String> = None;

        if markdown.trim().is_empty() && trimmed_tag.is_some() {
            attempted_filter_tag = trimmed_tag.map(|s| s.to_string());
            markdown = self
                .run_memory_query_pipeline(
                    &embedding,
                    qdrant_limit,
                    None,
                    trimmed_prefix,
                    options.min_score,
                    top_k,
                    options.max_total_chars,
                )
                .await?;
            if !markdown.trim().is_empty() {
                used_fallback = true;
                tracing::info!(
                    query = %query,
                    attempted_tag = ?attempted_filter_tag,
                    "memory:query used global search after tag filter returned no hits"
                );
            }
        }

        if markdown.trim().is_empty() && trimmed_prefix.is_some() {
            attempted_vault_prefix = trimmed_prefix.map(|s| s.to_string());
            markdown = self
                .run_memory_query_pipeline(
                    &embedding,
                    qdrant_limit,
                    trimmed_tag,
                    None,
                    options.min_score,
                    top_k,
                    options.max_total_chars,
                )
                .await?;
            if !markdown.trim().is_empty() {
                used_vault_prefix_fallback = true;
                tracing::info!(
                    query = %query,
                    attempted_prefix = ?attempted_vault_prefix,
                    "memory:query used search without vault_path_prefix after prefix matched no hits"
                );
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
            used_vault_prefix_fallback,
            attempted_vault_prefix: if used_vault_prefix_fallback {
                attempted_vault_prefix
            } else {
                None
            },
        })
    }

    async fn search_memory_query_recency(
        &self,
        query: &str,
        options: MemoryQueryOptions<'_>,
    ) -> Result<MemorySearchOutcome> {
        let trimmed_tag = options.filter_tag.map(str::trim).filter(|t| !t.is_empty());
        let trimmed_prefix = options
            .vault_path_prefix
            .map(str::trim)
            .filter(|t| !t.is_empty());

        let qdrant_limit = qdrant_oversample_limit(
            options.top_k,
            trimmed_prefix.is_some(),
            options.qdrant_oversample_cap,
            options.qdrant_oversample_multiplier,
            options.qdrant_oversample_min,
        );

        tracing::debug!(
            top_k = options.top_k,
            qdrant_limit,
            has_tag = trimmed_tag.is_some(),
            has_prefix = trimmed_prefix.is_some(),
            max_total_chars = options.max_total_chars,
            "memory:query search_memory_query_recency"
        );

        let top_k = (options.top_k as usize).max(1);

        let mut markdown = self
            .run_memory_recency_query_pipeline(
                qdrant_limit,
                trimmed_tag,
                trimmed_prefix,
                top_k,
                options.max_total_chars,
            )
            .await?;

        let mut used_fallback = false;
        let mut attempted_filter_tag: Option<String> = None;
        let mut used_vault_prefix_fallback = false;
        let mut attempted_vault_prefix: Option<String> = None;

        if markdown.trim().is_empty() && trimmed_tag.is_some() {
            attempted_filter_tag = trimmed_tag.map(|s| s.to_string());
            markdown = self
                .run_memory_recency_query_pipeline(
                    qdrant_limit,
                    None,
                    trimmed_prefix,
                    top_k,
                    options.max_total_chars,
                )
                .await?;
            if !markdown.trim().is_empty() {
                used_fallback = true;
                tracing::info!(
                    query = %query,
                    attempted_tag = ?attempted_filter_tag,
                    "memory:query recency used global scroll after tag filter returned no hits"
                );
            }
        }

        if markdown.trim().is_empty() && trimmed_prefix.is_some() {
            attempted_vault_prefix = trimmed_prefix.map(|s| s.to_string());
            markdown = self
                .run_memory_recency_query_pipeline(
                    qdrant_limit,
                    trimmed_tag,
                    None,
                    top_k,
                    options.max_total_chars,
                )
                .await?;
            if !markdown.trim().is_empty() {
                used_vault_prefix_fallback = true;
                tracing::info!(
                    query = %query,
                    attempted_prefix = ?attempted_vault_prefix,
                    "memory:query recency used scroll without vault_path_prefix after prefix matched no hits"
                );
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
            used_vault_prefix_fallback,
            attempted_vault_prefix: if used_vault_prefix_fallback {
                attempted_vault_prefix
            } else {
                None
            },
        })
    }

    async fn run_memory_recency_query_pipeline(
        &self,
        qdrant_limit: u64,
        qdrant_tag: Option<&str>,
        vault_prefix: Option<&str>,
        top_k: usize,
        max_total_chars: usize,
    ) -> Result<String> {
        let scroll_limit = u32::try_from(qdrant_limit.max(1)).unwrap_or(u32::MAX);
        let mut hits = self.scroll_recency_hits(scroll_limit, qdrant_tag).await?;
        hits = filter_hits_vault_prefix(hits, vault_prefix);
        hits = truncate_hits_preserve_order(hits, top_k);
        Ok(format_memory_hits_markdown(&hits, max_total_chars))
    }

    async fn scroll_recency_hits(
        &self,
        limit: u32,
        filter_tag: Option<&str>,
    ) -> Result<Vec<MemoryHit>> {
        if self.config.qdrant_collection_v2.is_empty() {
            return Ok(Vec::new());
        }

        let order_by = OrderBy {
            key: RECENCY_TS_PAYLOAD_KEY.to_string(),
            direction: Some(Direction::Desc as i32),
            start_from: None,
        };

        let mut scroll = ScrollPointsBuilder::new(self.config.qdrant_collection_v2.clone())
            .limit(limit.max(1))
            .with_payload(true)
            .order_by(order_by);

        if let Some(tag) = filter_tag.map(str::trim).filter(|t| !t.is_empty()) {
            scroll = scroll.filter(Filter::must([Condition::matches("tags", tag.to_string())]));
        }

        let scroll_response = self
            .client
            .scroll(scroll)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let mut out = Vec::new();
        for point in scroll_response.result {
            let Some(hit) = memory_hit_from_retrieved_payload(&point.payload) else {
                continue;
            };
            out.push(hit);
        }

        Ok(out)
    }

    async fn run_memory_query_pipeline(
        &self,
        embedding: &[f32],
        qdrant_limit: u64,
        qdrant_tag: Option<&str>,
        vault_prefix: Option<&str>,
        min_score: Option<f32>,
        top_k: usize,
        max_total_chars: usize,
    ) -> Result<String> {
        let mut hits = self
            .search_points_hits(embedding, qdrant_limit, qdrant_tag)
            .await?;
        hits = filter_hits_min_score(hits, min_score);
        hits = filter_hits_vault_prefix(hits, vault_prefix);
        hits = sort_and_limit_hits(hits, top_k);
        Ok(format_memory_hits_markdown(&hits, max_total_chars))
    }

    async fn search_points_hits(
        &self,
        embedding: &[f32],
        limit: u64,
        filter_tag: Option<&str>,
    ) -> Result<Vec<MemoryHit>> {
        let mut builder =
            SearchPointsBuilder::new(&self.config.qdrant_collection_v2, embedding.to_vec(), limit)
                .with_payload(true);

        if let Some(tag) = filter_tag.map(str::trim).filter(|t| !t.is_empty()) {
            builder = builder.filter(Filter::must([Condition::matches("tags", tag.to_string())]));
        }

        let search_result = self
            .client
            .search_points(builder)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let mut out = Vec::new();
        for point in search_result.result {
            let score = point.score;
            let payload = point.payload;
            let Some(text_val) = payload.get("text") else {
                continue;
            };
            let Some(qdrant_client::qdrant::value::Kind::StringValue(text)) = &text_val.kind else {
                continue;
            };
            let vault_key = payload.get("vault_key").and_then(|v| {
                if let Some(qdrant_client::qdrant::value::Kind::StringValue(s)) = &v.kind {
                    Some(s.clone())
                } else {
                    None
                }
            });
            out.push(MemoryHit {
                score,
                text: text.clone(),
                vault_key,
                recency_ts_ms: None,
            });
        }

        Ok(out)
    }
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn system_time_to_unix_ms(st: std::time::SystemTime) -> Option<u64> {
    st.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
}

fn truncate_hits_preserve_order(mut hits: Vec<MemoryHit>, top_k: usize) -> Vec<MemoryHit> {
    hits.truncate(top_k.max(1));
    hits
}

fn payload_integer_to_u64(val: &qdrant_client::qdrant::Value) -> Option<u64> {
    match &val.kind {
        Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)) if *i >= 0 => {
            u64::try_from(*i).ok()
        }
        Some(qdrant_client::qdrant::value::Kind::DoubleValue(d)) if *d >= 0.0 && d.is_finite() => {
            Some(*d as u64)
        }
        _ => None,
    }
}

fn memory_hit_from_retrieved_payload(
    payload: &std::collections::HashMap<String, qdrant_client::qdrant::Value>,
) -> Option<MemoryHit> {
    let text_val = payload.get("text")?;
    let Some(qdrant_client::qdrant::value::Kind::StringValue(text)) = &text_val.kind else {
        return None;
    };
    let vault_key = payload.get("vault_key").and_then(|v| {
        if let Some(qdrant_client::qdrant::value::Kind::StringValue(s)) = &v.kind {
            Some(s.clone())
        } else {
            None
        }
    });
    let recency_ts_ms = payload
        .get(RECENCY_TS_PAYLOAD_KEY)
        .and_then(payload_integer_to_u64);
    Some(MemoryHit {
        score: 1.0,
        text: text.clone(),
        vault_key,
        recency_ts_ms,
    })
}

fn qdrant_oversample_limit(
    top_k: u64,
    with_prefix: bool,
    cap: u64,
    multiplier: u64,
    min_when_prefix: u64,
) -> u64 {
    let k = top_k.max(1);
    let mult = multiplier.max(1);
    let floor = min_when_prefix.max(1);
    let cap = cap.max(1);
    let base = if with_prefix {
        k.saturating_mul(mult).max(floor)
    } else {
        k
    };
    base.min(cap)
}

fn filter_hits_min_score(hits: Vec<MemoryHit>, min_score: Option<f32>) -> Vec<MemoryHit> {
    let Some(th) = min_score else {
        return hits;
    };
    hits.into_iter().filter(|h| h.score >= th).collect()
}

fn filter_hits_vault_prefix(hits: Vec<MemoryHit>, prefix: Option<&str>) -> Vec<MemoryHit> {
    let Some(p) = prefix.map(str::trim).filter(|s| !s.is_empty()) else {
        return hits;
    };
    hits.into_iter()
        .filter(|h| {
            h.vault_key
                .as_deref()
                .map(|k| k.starts_with(p))
                .unwrap_or(false)
        })
        .collect()
}

fn sort_and_limit_hits(mut hits: Vec<MemoryHit>, top_k: usize) -> Vec<MemoryHit> {
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(top_k.max(1));
    hits
}

/// Formats ranked hits for the tool; trims to fit `max_total_chars`.
pub fn format_memory_hits_markdown(hits: &[MemoryHit], max_total_chars: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for h in hits {
        let header = if let Some(ts) = h.recency_ts_ms {
            format!("- (recency_ms: {ts}) ")
        } else {
            format!("- (score: {:.4}) ", h.score)
        };
        if used >= max_total_chars {
            break;
        }
        let remain = max_total_chars.saturating_sub(used);
        // Reserve one byte for newline so total line length never exceeds `remain`.
        if remain <= header.len().saturating_add(1) {
            break;
        }
        let body_budget = remain - header.len() - 1;
        let body = truncate_char_boundary(&h.text, body_budget);
        let line = format!("{header}{body}\n");
        used += line.len();
        out.push_str(&line);
    }
    out
}

fn vault_key_from_abs_path(vault_root: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let rel = path.strip_prefix(vault_root).ok()?;
    let key = rel.to_string_lossy().replace('\\', "/");
    if key.is_empty() {
        return None;
    }
    Some(key)
}

fn vault_key_is_ingest_eligible(key: &str) -> bool {
    if key.contains("/web/missions/") {
        return false;
    }
    VAULT_INGEST_SUBDIRS_V2
        .iter()
        .any(|subdir| key.starts_with(&format!("{subdir}/")) || key == *subdir)
}

fn synthesis_node_id_from_key(key: &str) -> Option<String> {
    let rest = key.strip_prefix("30_Synthesis/")?;
    let node_id = rest.split('/').next()?;
    if node_id.is_empty() {
        return None;
    }
    Some(node_id.to_string())
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

/// Extract a single string field from YAML frontmatter (e.g. `epistemic_status`).
fn extract_frontmatter_field(raw: &str, field: &str) -> Option<String> {
    if !raw.starts_with("---") {
        return None;
    }
    let after = &raw[3..];
    let end = after.find("---")?;
    let fm = &after[..end];
    let prefix = format!("{field}:");
    for line in fm.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix(&prefix) {
            let v = val.trim().trim_matches(|c| c == '"' || c == '\'');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
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
    use crate::engine::embedding::OllamaEmbedding;
    use ollama_rs::Ollama;
    use std::sync::Arc;

    fn dummy_embed_provider() -> Arc<dyn EmbeddingProvider> {
        let ollama = Arc::new(
            Ollama::builder()
                .host("http://localhost")
                .port(11434)
                .build(),
        );
        Arc::new(OllamaEmbedding::new(ollama, "nomic-embed-text".into()))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_semantic_brain_offline_returns_vector_db_offline() {
        let mut config = AppConfig::default();
        config.qdrant_url = "http://localhost:65535".to_string();

        let brain_result = SemanticBrain::new(Arc::new(config), dummy_embed_provider()).await;

        match brain_result {
            Err(FcpError::NetworkFault(_)) => (),
            _ => panic!("Expected NetworkFault error, got success instead"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_semantic_brain_connect_retries_exhaust_dead_port() {
        let mut config = AppConfig::default();
        config.qdrant_url = "http://127.0.0.1:65535".to_string();
        let brain_result =
            SemanticBrain::new_with_connect_retries(Arc::new(config), dummy_embed_provider(), 2, 1)
                .await;
        assert!(
            brain_result.is_err(),
            "expected failure after retries on dead port"
        );
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

    #[test]
    fn format_memory_hits_markdown_shows_recency_header() {
        let hits = vec![MemoryHit {
            score: 1.0,
            text: "hello".into(),
            vault_key: Some("30_Synthesis/x/r0001.md".into()),
            recency_ts_ms: Some(1_700_000_000_000),
        }];
        let md = format_memory_hits_markdown(&hits, 500);
        assert!(md.contains("recency_ms:"));
        assert!(!md.contains("score:"));
    }

    #[test]
    fn truncate_hits_preserve_order_keeps_sequence() {
        let hits = vec![
            MemoryHit {
                score: 1.0,
                text: "first".into(),
                vault_key: None,
                recency_ts_ms: Some(3),
            },
            MemoryHit {
                score: 1.0,
                text: "second".into(),
                vault_key: None,
                recency_ts_ms: Some(2),
            },
            MemoryHit {
                score: 1.0,
                text: "third".into(),
                vault_key: None,
                recency_ts_ms: Some(1),
            },
        ];
        let out = truncate_hits_preserve_order(hits, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "first");
        assert_eq!(out[1].text, "second");
    }

    #[test]
    fn system_time_to_unix_ms_positive() {
        let ts = system_time_to_unix_ms(std::time::UNIX_EPOCH);
        assert_eq!(ts, Some(0));
    }

    #[test]
    fn test_format_memory_hits_markdown_respects_budget() {
        let hits = vec![
            MemoryHit {
                score: 0.9,
                text: "a".repeat(100),
                vault_key: Some("40_User/x.md".to_string()),
                recency_ts_ms: None,
            },
            MemoryHit {
                score: 0.8,
                text: "b".repeat(100),
                vault_key: Some("40_User/y.md".to_string()),
                recency_ts_ms: None,
            },
        ];
        let md = format_memory_hits_markdown(&hits, 80);
        assert!(md.len() <= 80, "got {} bytes", md.len());
        assert!(md.contains("score:"));
    }

    #[test]
    fn test_filter_hits_min_score() {
        let hits = vec![
            MemoryHit {
                score: 0.5,
                text: "a".into(),
                vault_key: None,
                recency_ts_ms: None,
            },
            MemoryHit {
                score: 0.9,
                text: "b".into(),
                vault_key: None,
                recency_ts_ms: None,
            },
        ];
        let out = filter_hits_min_score(hits, Some(0.7));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "b");
    }

    #[test]
    fn test_filter_hits_vault_prefix() {
        let hits = vec![
            MemoryHit {
                score: 0.9,
                text: "a".into(),
                vault_key: Some("30_Persons/a.md".to_string()),
                recency_ts_ms: None,
            },
            MemoryHit {
                score: 0.8,
                text: "b".into(),
                vault_key: Some("40_User/b.md".to_string()),
                recency_ts_ms: None,
            },
            MemoryHit {
                score: 0.7,
                text: "c".into(),
                vault_key: None,
                recency_ts_ms: None,
            },
        ];
        let out = filter_hits_vault_prefix(hits, Some("30_Persons/"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "a");
    }

    #[test]
    fn vault_ingest_subdirs_omit_40_media_when_vision_disabled() {
        let mut config = AppConfig::default();
        config.vision.enabled = false;
        config.document_rag.enabled = false;
        let subdirs = vault_ingest_subdirs_for_config(&config);
        assert!(!subdirs.contains(&"40_MEDIA"));
        assert!(subdirs.contains(&"30_Synthesis"));

        config.document_rag.enabled = true;
        let subdirs = vault_ingest_subdirs_for_config(&config);
        assert!(subdirs.contains(&"40_MEDIA"));

        config.vision.enabled = true;
        let subdirs = vault_ingest_subdirs_for_config(&config);
        assert!(subdirs.contains(&"40_MEDIA"));
    }

    #[test]
    fn vault_key_from_abs_path_finds_40_media_json() {
        let root = std::path::Path::new("/vault/gem");
        let abs = root.join("40_MEDIA/abc123/media.json");
        let key = vault_key_from_abs_path(root, &abs).expect("key");
        assert_eq!(key, "40_MEDIA/abc123/media.json");
    }

    #[test]
    fn v2_ingest_subdirs_include_all_roots() {
        assert!(VAULT_INGEST_SUBDIRS_V2.contains(&"00_Invariants"));
        assert!(VAULT_INGEST_SUBDIRS_V2.contains(&"10_Topology"));
        assert!(VAULT_INGEST_SUBDIRS_V2.contains(&"20_Discourse"));
        assert!(VAULT_INGEST_SUBDIRS_V2.contains(&"30_Synthesis"));
        assert!(VAULT_INGEST_SUBDIRS_V2.contains(&"40_MEDIA"));
    }

    #[test]
    fn extract_frontmatter_field_gets_value() {
        let raw = "---\ntitle: \"test\"\nepistemic_status: \"stable\"\n---\nbody";
        assert_eq!(
            extract_frontmatter_field(raw, "epistemic_status"),
            Some("stable".to_string())
        );
    }

    #[test]
    fn extract_frontmatter_field_missing() {
        let raw = "---\ntitle: \"test\"\n---\nbody";
        assert_eq!(extract_frontmatter_field(raw, "epistemic_status"), None);
    }

    #[test]
    fn extract_frontmatter_field_no_frontmatter() {
        let raw = "just body text";
        assert_eq!(extract_frontmatter_field(raw, "anything"), None);
    }

    #[test]
    fn test_qdrant_oversample_limit() {
        let cap = 200;
        let mult = 25;
        let floor = 30;
        assert_eq!(qdrant_oversample_limit(5, false, cap, mult, floor), 5);
        assert_eq!(qdrant_oversample_limit(5, true, cap, mult, floor), 125);
        assert_eq!(qdrant_oversample_limit(100, true, cap, mult, floor), 200);
    }

    #[test]
    fn vector_dim_from_collection_info_reads_params_size() {
        use qdrant_client::qdrant::{
            CollectionConfig, CollectionInfo, CollectionParams, GetCollectionInfoResponse,
            VectorParams, VectorsConfig,
        };
        let vp = VectorParams {
            size: 384,
            distance: 0,
            hnsw_config: None,
            quantization_config: None,
            on_disk: None,
            datatype: None,
            multivector_config: None,
        };
        let mut vconf = VectorsConfig::default();
        vconf.config = Some(VectorSchemaConfig::Params(vp));
        let params = CollectionParams {
            shard_number: 1,
            on_disk_payload: false,
            vectors_config: Some(vconf),
            replication_factor: None,
            write_consistency_factor: None,
            read_fan_out_factor: None,
            sharding_method: None,
            sparse_vectors_config: None,
            read_fan_out_delay_ms: None,
        };
        let mut cc = CollectionConfig::default();
        cc.params = Some(params);
        let mut ci = CollectionInfo::default();
        ci.config = Some(cc);
        let mut info = GetCollectionInfoResponse::default();
        info.result = Some(ci);
        assert_eq!(vector_dim_from_collection_info(&info), Some(384));
    }

    #[test]
    fn dimension_mismatch_fails_fast() {
        let err = embedding_dims_agree_or_config_err(768, Some(384), "fcp_vault_v2_default")
            .expect_err("expected mismatch");
        match err {
            FcpError::Config(msg) => {
                assert!(msg.contains("768"), "{msg}");
                assert!(msg.contains("384"), "{msg}");
            }
            e => panic!("unexpected error: {:?}", e),
        }
    }
}
