//! Qdrant-backed chunked document store (`fcp_docs_{workspace}`).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, DeletePointsBuilder,
    Distance, FieldType, Filter, PointStruct, ScrollPointsBuilder, SearchPointsBuilder,
    UpsertPointsBuilder, VectorParamsBuilder,
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::fs;

use crate::config::AppConfig;
use crate::engine::EmbeddingProvider;
use crate::executive::error::{FcpError, Result};
use crate::ingest::{chunk_document, extract_text_from_path, truncate_char_boundary};
use crate::media::{
    CatalogInput, MediaType, catalog_abs_path, catalog_relative_path, upsert_catalog,
};
use crate::util::blob_store::sha256_hex;

const DOC_ID_PAYLOAD: &str = "doc_id";
const SOURCE_PATH_PAYLOAD: &str = "source_path";
const SOURCE_NAME_PAYLOAD: &str = "source_name";
const CHUNK_INDEX_PAYLOAD: &str = "chunk_index";
const TOTAL_CHUNKS_PAYLOAD: &str = "total_chunks";
const CONTENT_HASH_PAYLOAD: &str = "content_hash";
const INGESTED_AT_MS_PAYLOAD: &str = "ingested_at_ms";
const TEXT_PAYLOAD: &str = "text";

#[derive(Debug, Clone, Serialize)]
pub struct IngestReceipt {
    pub doc_id: String,
    pub content_hash: String,
    pub source_path: String,
    pub source_name: String,
    pub total_chunks: u32,
    pub preview_head: String,
    pub catalog_path: String,
    pub skipped_unchanged: bool,
}

#[derive(Debug, Clone)]
pub struct DocumentChunk {
    pub text: String,
    pub doc_id: String,
    pub source_path: String,
    pub source_name: String,
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub content_hash: String,
    pub ingested_at_ms: u64,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentSummary {
    pub doc_id: String,
    pub source_path: String,
    pub source_name: String,
    pub total_chunks: u32,
    pub ingested_at_ms: u64,
    pub content_hash: String,
}

#[derive(Clone)]
pub struct DocumentStore {
    client: Arc<Qdrant>,
    embed: Arc<dyn EmbeddingProvider>,
    config: Arc<AppConfig>,
}

impl DocumentStore {
    pub async fn new(config: Arc<AppConfig>, embed: Arc<dyn EmbeddingProvider>) -> Result<Self> {
        let client = Qdrant::from_url(&config.qdrant_url)
            .build()
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let collection = &config.qdrant_docs_collection;
        let exists = client
            .collection_exists(collection)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        if !exists {
            // Width from the active embedding provider (see `SemanticBrain::new` rationale).
            let dims = embed.dimensions() as u64;
            client
                .create_collection(
                    CreateCollectionBuilder::new(collection)
                        .vectors_config(VectorParamsBuilder::new(dims, Distance::Cosine)),
                )
                .await
                .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
            tracing::info!(collection = %collection, dims, "Created Qdrant document collection");
        }

        let store = Self {
            client: Arc::new(client),
            embed,
            config,
        };
        store.ensure_keyword_index(DOC_ID_PAYLOAD).await?;
        store.ensure_keyword_index(SOURCE_PATH_PAYLOAD).await?;
        Ok(store)
    }

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
                Ok(store) => return Ok(store),
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        attempts,
                        error = %e,
                        "DocumentStore gRPC connect attempt failed"
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
                "DocumentStore connect retries exhausted".into(),
            )),
        }
    }

    async fn ensure_keyword_index(&self, field: &str) -> Result<()> {
        let collection = &self.config.qdrant_docs_collection;
        if collection.is_empty() {
            return Ok(());
        }
        let builder = CreateFieldIndexCollectionBuilder::new(
            collection.clone(),
            field.to_string(),
            FieldType::Keyword,
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
                        "document payload index {field}: {msg}"
                    )))
                }
            }
        }
    }

    pub fn chunk_point_id(source_path: &str, chunk_index: u32) -> String {
        let key = format!("{source_path}:chunk:{chunk_index}");
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes()).to_string()
    }

    /// Deterministic doc_id derived from the source path. Survives Qdrant wipes
    /// and re-ingests because the same file always maps to the same UUID.
    pub fn deterministic_doc_id(source_path: &str) -> String {
        let key = format!("{source_path}:doc_id");
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes()).to_string()
    }

    pub async fn ingest_document(
        &self,
        vault_root: &Path,
        relative_path: &str,
        source_label: Option<&str>,
    ) -> Result<IngestReceipt> {
        let source_path = relative_path.replace('\\', "/");
        let abs = vault_root.join(&source_path);
        if !abs.is_file() {
            return Err(FcpError::ToolFault {
                tool_name: "doc:ingest".into(),
                reason: format!("file not found: {source_path}"),
            });
        }

        let raw = fs::read(&abs).await.map_err(FcpError::Io)?;
        let max_bytes = self.config.document_rag.max_file_bytes;
        if raw.len() as u64 > max_bytes {
            return Err(FcpError::ToolFault {
                tool_name: "doc:ingest".into(),
                reason: format!(
                    "file {} bytes exceeds document_rag.max_file_bytes {}",
                    raw.len(),
                    max_bytes
                ),
            });
        }

        let content_hash = sha256_hex(&raw);
        if let Some(existing) = self.existing_ingest_for_path(&source_path).await? {
            if existing.content_hash == content_hash {
                return Ok(IngestReceipt {
                    doc_id: existing.doc_id,
                    content_hash: content_hash.clone(),
                    source_path: existing.source_path,
                    source_name: existing.source_name,
                    total_chunks: existing.total_chunks,
                    preview_head: existing.preview_head,
                    catalog_path: catalog_relative_path(&content_hash),
                    skipped_unchanged: true,
                });
            }
            self.delete_chunks_for_path(&source_path).await?;
        }

        tracing::info!(
            event = "fcp.document_ingest.phase",
            phase = "extract_text",
            path = %source_path,
            bytes = raw.len(),
            "Extracting text from document"
        );
        let text = extract_text_from_path(&abs).await?;
        if text.trim().is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: "doc:ingest".into(),
                reason: "extracted text is empty".into(),
            });
        }

        tracing::info!(
            event = "fcp.document_ingest.phase",
            phase = "chunking",
            path = %source_path,
            text_chars = text.len(),
            "Chunking extracted text"
        );
        let chunk_cfg = self.config.document_rag.resolved_chunk_config();
        let chunks = chunk_document(&text, &chunk_cfg);
        if chunks.is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: "doc:ingest".into(),
                reason: "no chunks produced from document text".into(),
            });
        }

        let max_chunks = self.config.document_rag.max_chunks_per_doc.max(1);
        if chunks.len() as u32 > max_chunks {
            return Err(FcpError::ToolFault {
                tool_name: "doc:ingest".into(),
                reason: format!(
                    "document produced {} chunks; exceeds document_rag.max_chunks_per_doc {}",
                    chunks.len(),
                    max_chunks
                ),
            });
        }

        let doc_id = Self::deterministic_doc_id(&source_path);
        let source_name = source_label
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                abs.file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| source_path.clone());
        let total_chunks = u32::try_from(chunks.len()).map_err(|_| FcpError::ToolFault {
            tool_name: "doc:ingest".into(),
            reason: "chunk count overflow".into(),
        })?;
        let ingested_at_ms = unix_ms_now();
        let preview_head = truncate_char_boundary(
            chunks.first().map(String::as_str).unwrap_or(""),
            400,
        );

        tracing::info!(
            event = "fcp.document_ingest.phase",
            phase = "embedding",
            path = %source_path,
            total_chunks,
            "Starting chunk embedding loop"
        );
        let mut points = Vec::with_capacity(chunks.len());
        for (idx, chunk_text) in chunks.iter().enumerate() {
            let chunk_index = u32::try_from(idx).map_err(|_| FcpError::ToolFault {
                tool_name: "doc:ingest".into(),
                reason: "chunk index overflow".into(),
            })?;
            let embedding =
                embed_with_bisect_retry(self.embed.as_ref(), chunk_text, chunk_index).await?;
            let point_id = Self::chunk_point_id(&source_path, chunk_index);
            let mut payload: HashMap<String, Value> = HashMap::new();
            payload.insert(TEXT_PAYLOAD.into(), json!(chunk_text));
            payload.insert(DOC_ID_PAYLOAD.into(), json!(doc_id));
            payload.insert(SOURCE_PATH_PAYLOAD.into(), json!(source_path));
            payload.insert(SOURCE_NAME_PAYLOAD.into(), json!(source_name));
            payload.insert(CHUNK_INDEX_PAYLOAD.into(), json!(chunk_index));
            payload.insert(TOTAL_CHUNKS_PAYLOAD.into(), json!(total_chunks));
            payload.insert(CONTENT_HASH_PAYLOAD.into(), json!(content_hash));
            payload.insert(INGESTED_AT_MS_PAYLOAD.into(), json!(ingested_at_ms));
            points.push(PointStruct::new(point_id, embedding, payload));
        }

        self.client
            .upsert_points(UpsertPointsBuilder::new(
                &self.config.qdrant_docs_collection,
                points,
            ))
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let description = preview_head.clone();
        let mut type_fields = BTreeMap::new();
        type_fields.insert("doc_id".into(), json!(doc_id));
        type_fields.insert("total_chunks".into(), json!(total_chunks));
        let extension = abs
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        type_fields.insert("format".into(), json!(extension));
        type_fields.insert("char_count".into(), json!(text.chars().count()));
        type_fields.insert(
            "ingested_at".into(),
            json!(chrono::Utc::now().to_rfc3339()),
        );

        upsert_catalog(
            vault_root,
            CatalogInput {
                relative_path: source_path.clone(),
                title: source_name.clone(),
                media_type: Some(MediaType::Document),
                tags: Vec::new(),
                description,
                user_notes: String::new(),
                uploaded_at: None,
                type_fields,
            },
        )
        .await?;

        if self.config.document_rag.cleanup_source_after_ingest {
            let upload_prefix = self
                .config
                .web_ui
                .uploads
                .files
                .upload_dir
                .trim_end_matches('/');
            if source_path.starts_with(upload_prefix) {
                if let Err(e) = fs::remove_file(&abs).await {
                    tracing::warn!(
                        event = "fcp.document_ingest.source_cleanup_failed",
                        path = %source_path,
                        error = %e,
                        "Post-ingest source file cleanup failed"
                    );
                } else {
                    tracing::info!(
                        event = "fcp.document_ingest.source_cleaned",
                        path = %source_path,
                        "Deleted source file after successful ingest"
                    );
                }
            }
        }

        Ok(IngestReceipt {
            doc_id,
            content_hash: content_hash.clone(),
            source_path,
            source_name,
            total_chunks,
            preview_head,
            catalog_path: catalog_relative_path(&content_hash),
            skipped_unchanged: false,
        })
    }

    pub async fn re_ingest(
        &self,
        vault_root: &Path,
        relative_path: &str,
        source_label: Option<&str>,
    ) -> Result<IngestReceipt> {
        let source_path = relative_path.replace('\\', "/");
        self.delete_chunks_for_path(&source_path).await?;
        self.ingest_document(vault_root, &source_path, source_label)
            .await
    }

    pub async fn query(
        &self,
        text: &str,
        top_k: u32,
        doc_id_filter: Option<&str>,
        min_score: Option<f32>,
        max_total_chars: usize,
    ) -> Result<Vec<DocumentChunk>> {
        let embedding = self.embed.embed(text).await?;
        let limit = u64::from(top_k.max(1));

        let mut builder = SearchPointsBuilder::new(
            &self.config.qdrant_docs_collection,
            embedding,
            limit,
        )
        .with_payload(true);

        if let Some(doc_id) = doc_id_filter.map(str::trim).filter(|s| !s.is_empty()) {
            builder = builder.filter(Filter::must([Condition::matches(
                DOC_ID_PAYLOAD,
                doc_id.to_string(),
            )]));
        }

        let search_result = self
            .client
            .search_points(builder)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let min = min_score.unwrap_or(self.config.document_rag.query_min_score);
        let mut out = Vec::new();
        let mut used_chars = 0usize;

        for point in search_result.result {
            if point.score < min {
                continue;
            }
            let Some(chunk) = document_chunk_from_payload(point.score, &point.payload) else {
                continue;
            };
            let add_len = chunk.text.len();
            if used_chars + add_len > max_total_chars && !out.is_empty() {
                break;
            }
            used_chars += add_len;
            out.push(chunk);
        }

        Ok(out)
    }

    pub async fn list_documents(&self) -> Result<Vec<DocumentSummary>> {
        let mut scroll = ScrollPointsBuilder::new(self.config.qdrant_docs_collection.clone())
            .limit(256)
            .with_payload(true);

        let mut by_doc: HashMap<String, DocumentSummary> = HashMap::new();

        loop {
            let response = self
                .client
                .scroll(scroll.clone())
                .await
                .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
            if response.result.is_empty() {
                break;
            }
            for point in &response.result {
                if let Some(chunk) = document_chunk_from_payload(0.0, &point.payload) {
                    by_doc
                        .entry(chunk.doc_id.clone())
                        .and_modify(|s| {
                            if chunk.ingested_at_ms > s.ingested_at_ms {
                                *s = summary_from_chunk(&chunk);
                            }
                        })
                        .or_insert_with(|| summary_from_chunk(&chunk));
                }
            }
            let Some(next) = response.next_page_offset else {
                break;
            };
            scroll = scroll.offset(next);
        }

        let mut docs: Vec<DocumentSummary> = by_doc.into_values().collect();
        docs.sort_by(|a, b| b.ingested_at_ms.cmp(&a.ingested_at_ms));
        Ok(docs)
    }

    pub async fn delete_document(&self, vault_root: &Path, doc_id: &str) -> Result<u32> {
        let doc_id = doc_id.trim();
        if doc_id.is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: "doc:delete".into(),
                reason: "doc_id is required".into(),
            });
        }

        let chunks = self.chunks_for_doc_id(doc_id).await?;
        if chunks.is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: "doc:delete".into(),
                reason: format!("no document found for doc_id {doc_id}"),
            });
        }

        let count = u32::try_from(chunks.len()).map_err(|_| FcpError::ToolFault {
            tool_name: "doc:delete".into(),
            reason: "chunk count overflow".into(),
        })?;
        let content_hash = chunks
            .first()
            .map(|c| c.content_hash.clone())
            .unwrap_or_default();

        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.config.qdrant_docs_collection)
                    .points(Filter::must([Condition::matches(
                        DOC_ID_PAYLOAD,
                        doc_id.to_string(),
                    )]))
                    .wait(true),
            )
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        if !content_hash.is_empty() {
            let catalog_path = catalog_abs_path(vault_root, &content_hash);
            if catalog_path.is_file() {
                fs::remove_file(&catalog_path).await.map_err(FcpError::Io)?;
            }
            let vault_key = catalog_relative_path(&content_hash);
            let point_id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, vault_key.as_bytes())
                .to_string();
            if !self.config.qdrant_collection_v2.is_empty() {
                let _ = self
                    .client
                    .delete_points(
                        DeletePointsBuilder::new(&self.config.qdrant_collection_v2)
                            .points(vec![point_id])
                            .wait(true),
                    )
                    .await;
            }
        }

        Ok(count)
    }

    /// Paginated sequential chunk reader: returns chunks `[start .. start+count)` for a
    /// document, sorted by `chunk_index`. Used by the `doc:read` tool.
    pub async fn read_chunks_page(
        &self,
        doc_id: &str,
        start: u32,
        count: u32,
    ) -> Result<Vec<DocumentChunk>> {
        let doc_id = doc_id.trim();
        if doc_id.is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: "doc:read".into(),
                reason: "doc_id is required".into(),
            });
        }
        let mut all = self.chunks_for_doc_id(doc_id).await?;
        if all.is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: "doc:read".into(),
                reason: format!(
                    "no document found for doc_id {doc_id}. \
                     The document may need to be re-ingested (use doc:ingest with the original file path). \
                     Use doc:list to see currently available documents."
                ),
            });
        }
        all.sort_by_key(|c| c.chunk_index);
        let end = (start + count).min(all.last().map(|c| c.chunk_index + 1).unwrap_or(0));
        Ok(all
            .into_iter()
            .filter(|c| c.chunk_index >= start && c.chunk_index < end)
            .collect())
    }

    async fn existing_ingest_for_path(
        &self,
        source_path: &str,
    ) -> Result<Option<ExistingIngest>> {
        let filter = Filter::must([Condition::matches(
            SOURCE_PATH_PAYLOAD,
            source_path.to_string(),
        )]);
        let response = self
            .client
            .scroll(
                ScrollPointsBuilder::new(self.config.qdrant_docs_collection.clone())
                    .filter(filter)
                    .limit(1)
                    .with_payload(true),
            )
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        let Some(point) = response.result.first() else {
            return Ok(None);
        };
        let Some(chunk) = document_chunk_from_payload(0.0, &point.payload) else {
            return Ok(None);
        };
        Ok(Some(ExistingIngest {
            doc_id: chunk.doc_id,
            content_hash: chunk.content_hash,
            source_path: chunk.source_path,
            source_name: chunk.source_name,
            total_chunks: chunk.total_chunks,
            preview_head: truncate_char_boundary(&chunk.text, 400),
        }))
    }

    async fn delete_chunks_for_path(&self, source_path: &str) -> Result<()> {
        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.config.qdrant_docs_collection)
                    .points(Filter::must([Condition::matches(
                        SOURCE_PATH_PAYLOAD,
                        source_path.to_string(),
                    )]))
                    .wait(true),
            )
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
        Ok(())
    }

    async fn chunks_for_doc_id(&self, doc_id: &str) -> Result<Vec<DocumentChunk>> {
        let filter = Filter::must([Condition::matches(
            DOC_ID_PAYLOAD,
            doc_id.to_string(),
        )]);
        let response = self
            .client
            .scroll(
                ScrollPointsBuilder::new(self.config.qdrant_docs_collection.clone())
                    .filter(filter)
                    .limit(512)
                    .with_payload(true),
            )
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;

        Ok(response
            .result
            .iter()
            .filter_map(|p| document_chunk_from_payload(0.0, &p.payload))
            .collect())
    }

    /// Boot-time reconciliation: scan `40_MEDIA` for document cards whose `doc_id`
    /// no longer maps to any chunks in Qdrant.  Returns the file_paths that need
    /// re-ingest so the caller can queue them.
    pub async fn reconcile_stale_doc_ids(
        &self,
        vault_root: &Path,
    ) -> Vec<String> {
        let media_dir = vault_root.join("40_MEDIA");
        let mut stale_paths: Vec<String> = Vec::new();

        let mut top_entries = match tokio::fs::read_dir(&media_dir).await {
            Ok(e) => e,
            Err(_) => return stale_paths,
        };

        while let Ok(Some(entry)) = top_entries.next_entry().await {
            let json_path = entry.path().join("media.json");
            if !json_path.is_file() {
                continue;
            }
            let raw = match tokio::fs::read_to_string(&json_path).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            let card: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let media_type = card.get("media_type").and_then(|v| v.as_str()).unwrap_or("");
            if media_type != "document" {
                continue;
            }

            let file_path = match card.get("file_path").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None => continue,
            };

            let stored_doc_id = card
                .get("type_fields")
                .and_then(|tf| tf.get("doc_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if stored_doc_id.is_empty() {
                continue;
            }

            let chunks = match self.chunks_for_doc_id(stored_doc_id).await {
                Ok(c) => c,
                Err(_) => Vec::new(),
            };

            if chunks.is_empty() {
                tracing::warn!(
                    file_path = %file_path,
                    stale_doc_id = %stored_doc_id,
                    "Boot reconciliation: document card references missing doc_id in Qdrant"
                );
                stale_paths.push(file_path);
            }
        }

        stale_paths
    }
}

#[derive(Debug, Clone)]
struct ExistingIngest {
    doc_id: String,
    content_hash: String,
    source_path: String,
    source_name: String,
    total_chunks: u32,
    preview_head: String,
}

fn summary_from_chunk(chunk: &DocumentChunk) -> DocumentSummary {
    DocumentSummary {
        doc_id: chunk.doc_id.clone(),
        source_path: chunk.source_path.clone(),
        source_name: chunk.source_name.clone(),
        total_chunks: chunk.total_chunks,
        ingested_at_ms: chunk.ingested_at_ms,
        content_hash: chunk.content_hash.clone(),
    }
}

fn document_chunk_from_payload(
    score: f32,
    payload: &HashMap<String, qdrant_client::qdrant::Value>,
) -> Option<DocumentChunk> {
    let text = qdrant_payload_string(payload, TEXT_PAYLOAD)?;
    Some(DocumentChunk {
        text,
        doc_id: qdrant_payload_string(payload, DOC_ID_PAYLOAD)?,
        source_path: qdrant_payload_string(payload, SOURCE_PATH_PAYLOAD)?,
        source_name: qdrant_payload_string(payload, SOURCE_NAME_PAYLOAD)?,
        chunk_index: qdrant_payload_u32(payload, CHUNK_INDEX_PAYLOAD)?,
        total_chunks: qdrant_payload_u32(payload, TOTAL_CHUNKS_PAYLOAD)?,
        content_hash: qdrant_payload_string(payload, CONTENT_HASH_PAYLOAD)?,
        ingested_at_ms: qdrant_payload_u64(payload, INGESTED_AT_MS_PAYLOAD).unwrap_or(0),
        score,
    })
}

fn qdrant_payload_string(
    payload: &HashMap<String, qdrant_client::qdrant::Value>,
    key: &str,
) -> Option<String> {
    let val = payload.get(key)?;
    if let Some(qdrant_client::qdrant::value::Kind::StringValue(s)) = &val.kind {
        Some(s.clone())
    } else {
        None
    }
}

fn qdrant_payload_u32(
    payload: &HashMap<String, qdrant_client::qdrant::Value>,
    key: &str,
) -> Option<u32> {
    let val = payload.get(key)?;
    match &val.kind {
        Some(qdrant_client::qdrant::value::Kind::IntegerValue(n)) => u32::try_from(*n).ok(),
        Some(qdrant_client::qdrant::value::Kind::DoubleValue(n)) => {
            u32::try_from(*n as i64).ok()
        }
        _ => None,
    }
}

fn qdrant_payload_u64(
    payload: &HashMap<String, qdrant_client::qdrant::Value>,
    key: &str,
) -> Option<u64> {
    let val = payload.get(key)?;
    match &val.kind {
        Some(qdrant_client::qdrant::value::Kind::IntegerValue(n)) => u64::try_from(*n).ok(),
        Some(qdrant_client::qdrant::value::Kind::DoubleValue(n)) => {
            u64::try_from(*n as i64).ok()
        }
        _ => None,
    }
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Embed a chunk with automatic bisection retry on failure.
///
/// When the embed server rejects a chunk (e.g. exceeds physical batch token limit), this
/// splits the text at the midpoint char boundary, embeds each half independently, and
/// returns the L2-normalized mean of both vectors. At most one bisection level (2 halves);
/// if both halves also fail, we propagate the error.
async fn embed_with_bisect_retry(
    embed: &dyn EmbeddingProvider,
    text: &str,
    chunk_index: u32,
) -> Result<Vec<f32>> {
    match embed.embed(text).await {
        Ok(v) => return Ok(v),
        Err(first_err) => {
            let err_msg = first_err.to_string().to_lowercase();
            let retriable = err_msg.contains("too large")
                || err_msg.contains("too many tokens")
                || err_msg.contains("500")
                || err_msg.contains("batch")
                || err_msg.contains("content length");
            if !retriable || text.len() < 64 {
                return Err(FcpError::EmbeddingFault(format!(
                    "chunk {chunk_index} embed failed: {first_err}"
                )));
            }
            tracing::warn!(
                chunk_index,
                text_len = text.len(),
                error = %first_err,
                "Embed failed — bisecting chunk and retrying"
            );
        }
    }

    let mid = text.len() / 2;
    let split_at = find_char_boundary_near(text, mid);
    let left = &text[..split_at];
    let right = &text[split_at..];

    let left_vec = embed.embed(left).await.map_err(|e| {
        FcpError::EmbeddingFault(format!(
            "chunk {chunk_index} bisect-left embed failed: {e}"
        ))
    })?;
    let right_vec = embed.embed(right).await.map_err(|e| {
        FcpError::EmbeddingFault(format!(
            "chunk {chunk_index} bisect-right embed failed: {e}"
        ))
    })?;

    Ok(mean_normalize(&left_vec, &right_vec))
}

fn find_char_boundary_near(text: &str, target: usize) -> usize {
    let mut pos = target.min(text.len());
    while pos > 0 && !text.is_char_boundary(pos) {
        pos -= 1;
    }
    if pos == 0 && !text.is_char_boundary(0) {
        return text.len();
    }
    pos
}

fn mean_normalize(a: &[f32], b: &[f32]) -> Vec<f32> {
    let mut out: Vec<f32> = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x + y) * 0.5)
        .collect();
    let norm: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut out {
            *x /= norm;
        }
    }
    out
}

pub fn format_query_results_markdown(chunks: &[DocumentChunk]) -> String {
    if chunks.is_empty() {
        return "No matching document chunks.".to_string();
    }
    let mut out = String::from("## Document search results\n\n");
    for chunk in chunks {
        out.push_str(&format!(
            "### {} (chunk {}/{}, score {:.3})\n",
            chunk.source_name,
            chunk.chunk_index + 1,
            chunk.total_chunks,
            chunk.score
        ));
        out.push_str(&format!(
            "- **doc_id:** `{}`\n- **path:** `{}`\n\n",
            chunk.doc_id, chunk.source_path
        ));
        out.push_str(chunk.text.trim());
        out.push_str("\n\n---\n\n");
    }
    out
}

pub fn format_list_documents_markdown(summaries: &[DocumentSummary]) -> String {
    if summaries.is_empty() {
        return "No ingested documents.".to_string();
    }
    let mut out = String::from("| doc_id | source | chunks | ingested |\n");
    out.push_str("|--------|--------|--------|----------|\n");
    for s in summaries {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            s.doc_id, s.source_name, s.total_chunks, s.ingested_at_ms
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::config::{AppConfig, DocumentRagConfig};
    use crate::engine::EmbeddingProvider;

    struct DeterministicEmbed {
        dims: usize,
    }

    #[async_trait]
    impl EmbeddingProvider for DeterministicEmbed {
        async fn embed(&self, text: &str) -> crate::executive::error::Result<Vec<f32>> {
            let mut v = vec![0.0f32; self.dims];
            for (i, b) in text.bytes().enumerate() {
                v[i % self.dims] += f32::from(b) / 255.0;
            }
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for x in &mut v {
                    *x /= norm;
                }
            }
            Ok(v)
        }

        fn dimensions(&self) -> usize {
            self.dims
        }
    }

    #[test]
    fn chunk_point_id_is_deterministic() {
        let a = DocumentStore::chunk_point_id("99_USER_UPLOADED/files/x.md", 0);
        let b = DocumentStore::chunk_point_id("99_USER_UPLOADED/files/x.md", 0);
        assert_eq!(a, b);
        let c = DocumentStore::chunk_point_id("99_USER_UPLOADED/files/x.md", 1);
        assert_ne!(a, c);
    }

    #[test]
    fn deterministic_doc_id_is_stable() {
        let a = DocumentStore::deterministic_doc_id("99_USER_UPLOADED/files/paper.pdf");
        let b = DocumentStore::deterministic_doc_id("99_USER_UPLOADED/files/paper.pdf");
        assert_eq!(a, b);
        let c = DocumentStore::deterministic_doc_id("99_USER_UPLOADED/files/other.pdf");
        assert_ne!(a, c);
    }

    #[test]
    fn deterministic_doc_id_differs_from_chunk_point_id() {
        let doc = DocumentStore::deterministic_doc_id("99_USER_UPLOADED/files/x.md");
        let chunk = DocumentStore::chunk_point_id("99_USER_UPLOADED/files/x.md", 0);
        assert_ne!(doc, chunk);
    }

    #[test]
    fn format_query_includes_source() {
        let chunks = vec![DocumentChunk {
            text: "Hello passage.".into(),
            doc_id: "abc".into(),
            source_path: "99_USER_UPLOADED/files/a.md".into(),
            source_name: "report.md".into(),
            chunk_index: 0,
            total_chunks: 2,
            content_hash: "deadbeef".into(),
            ingested_at_ms: 1,
            score: 0.87,
        }];
        let md = format_query_results_markdown(&chunks);
        assert!(md.contains("report.md"));
        assert!(md.contains("Hello passage"));
        assert!(md.contains("abc"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ingest_markdown_query_roundtrip_when_qdrant_online() {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let mut config = AppConfig::default();
        config.qdrant_url = "http://localhost:6334".into();
        config.workspace = format!("doc_rag_test_{suffix}");
        config.qdrant_docs_collection = format!("fcp_docs_{}", config.workspace);
        config.qdrant_collection_v2 = format!("fcp_vault_v2_{}", config.workspace);
        config.document_rag = DocumentRagConfig::default();

        let embed: Arc<dyn EmbeddingProvider> = Arc::new(DeterministicEmbed { dims: 768 });
        let store = match DocumentStore::new(Arc::new(config), embed).await {
            Ok(s) => s,
            Err(_) => return,
        };

        let dir = tempfile::tempdir().expect("tempdir");
        let rel = "99_USER_UPLOADED/files/report.md";
        let abs = dir.path().join(rel);
        tokio::fs::create_dir_all(abs.parent().expect("parent"))
            .await
            .expect("mkdir");
        let body = "# Report\n\nThe quarterly revenue grew by twelve percent.\n\nCosts remained flat.";
        tokio::fs::write(&abs, body).await.expect("write");

        let receipt = store
            .ingest_document(dir.path(), rel, Some("Q Report"))
            .await
            .expect("ingest");
        assert!(!receipt.skipped_unchanged);
        assert!(receipt.total_chunks >= 1);

        let hits = store
            .query("revenue growth", 5, None, Some(0.0), 8_000)
            .await
            .expect("query");
        assert!(!hits.is_empty());
        assert!(hits[0].text.to_lowercase().contains("revenue"));
        assert_eq!(hits[0].doc_id, receipt.doc_id);
        assert_eq!(hits[0].source_name, "Q Report");

        let again = store
            .ingest_document(dir.path(), rel, Some("Q Report"))
            .await
            .expect("re-ingest unchanged");
        assert!(again.skipped_unchanged);

        store
            .delete_document(dir.path(), &receipt.doc_id)
            .await
            .expect("delete");
    }
}
