//! Read/write `40_MEDIA/{content_hash}/media.json` and Qdrant embed text.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::fs;

use crate::executive::error::{FcpError, Result};
use crate::memory::semantic::vault_embed_text;
use crate::util::blob_store::{sha256_hex_file, unix_now_secs};

use super::paths::MEDIA_CATALOG_DIR;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    Audio,
    Document,
}

impl MediaType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Document => "document",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Image => "image/jpeg",
            Self::Audio => "audio/wav",
            Self::Document => "application/octet-stream",
        }
    }
}

pub fn infer_media_type_from_path(relative_path: &str) -> Option<MediaType> {
    let norm = relative_path.replace('\\', "/");
    if norm.contains("/images/") || norm.starts_with("99_USER_UPLOADED/images/") {
        return Some(MediaType::Image);
    }
    if norm.contains("/audio/") || norm.starts_with("99_USER_UPLOADED/audio/") {
        return Some(MediaType::Audio);
    }
    if norm.contains("/files/") || norm.starts_with("99_USER_UPLOADED/files/") {
        return Some(MediaType::Document);
    }
    None
}

/// Canonical on-disk catalog record (`40_MEDIA/{content_hash}/media.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaCard {
    pub schema_version: u32,
    pub content_hash: String,
    #[serde(rename = "media_type")]
    pub media_type: MediaType,
    pub file_path: String,
    pub mime_type: String,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub user_notes: String,
    pub uploaded_at: u64,
    pub cataloged_at: u64,
    pub updated_at: u64,
    pub source: String,
    #[serde(default)]
    pub type_fields: BTreeMap<String, Value>,
}

pub fn catalog_relative_path(content_hash: &str) -> String {
    format!("{MEDIA_CATALOG_DIR}/{content_hash}/media.json")
}

pub fn catalog_abs_path(workspace_root: &Path, content_hash: &str) -> PathBuf {
    workspace_root.join(catalog_relative_path(content_hash))
}

fn format_embed_date(unix_secs: u64) -> String {
    if unix_secs == 0 {
        return "unknown".to_string();
    }
    DateTime::<Utc>::from_timestamp(unix_secs as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn build_embed_text(card: &MediaCard) -> String {
    let mut body = String::new();
    body.push_str("Media type: ");
    body.push_str(card.media_type.as_str());
    body.push('\n');
    body.push_str("Uploaded: ");
    body.push_str(&format_embed_date(card.uploaded_at));
    body.push('\n');
    body.push_str("Cataloged: ");
    body.push_str(&format_embed_date(card.cataloged_at));
    body.push('\n');
    body.push_str("File: ");
    body.push_str(&card.file_path);
    body.push_str("\n\nDescription:\n");
    body.push_str(card.description.trim());
    if !card.user_notes.trim().is_empty() {
        body.push_str("\n\nNotes:\n");
        body.push_str(card.user_notes.trim());
    }
    vault_embed_text(Some(&card.title), &card.tags, &body)
}

pub async fn content_hash_for_file(workspace_root: &Path, relative_path: &str) -> Result<String> {
    let norm = relative_path.replace('\\', "/");
    let abs = workspace_root.join(&norm);
    if let Some(stem) = abs
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Ok(stem.to_string());
    }
    sha256_hex_file(&abs).await
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogInput {
    pub relative_path: String,
    pub title: String,
    #[serde(default)]
    pub media_type: Option<MediaType>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub user_notes: String,
    #[serde(default)]
    pub uploaded_at: Option<u64>,
    #[serde(default)]
    pub type_fields: BTreeMap<String, Value>,
}

pub async fn upsert_catalog(
    workspace_root: &Path,
    input: CatalogInput,
) -> Result<MediaCard> {
    let file_path = input.relative_path.replace('\\', "/");
    let media_type = input
        .media_type
        .or_else(|| infer_media_type_from_path(&file_path))
        .ok_or_else(|| {
            FcpError::ToolFault {
                tool_name: "media:catalog".into(),
                reason: "could not infer media_type from path".into(),
            }
        })?;
    let content_hash = content_hash_for_file(workspace_root, &file_path).await?;
    let now = unix_now_secs();
    let catalog_path = catalog_abs_path(workspace_root, &content_hash);
    let existing = load_card_from_path(&catalog_path).await?;

    let uploaded_at = input.uploaded_at.unwrap_or_else(|| {
        existing
            .as_ref()
            .map(|c| c.uploaded_at)
            .unwrap_or(now)
    });
    let cataloged_at = existing
        .as_ref()
        .map(|c| c.cataloged_at)
        .unwrap_or(now);

    let card = MediaCard {
        schema_version: 1,
        content_hash: content_hash.clone(),
        media_type,
        file_path: file_path.clone(),
        mime_type: media_type.mime_type().to_string(),
        title: input.title,
        tags: normalize_tags(input.tags),
        description: input.description,
        user_notes: input.user_notes,
        uploaded_at,
        cataloged_at,
        updated_at: now,
        source: "media:catalog".to_string(),
        type_fields: input.type_fields,
    };

    if let Some(parent) = catalog_path.parent() {
        fs::create_dir_all(parent).await.map_err(FcpError::Io)?;
    }
    let body = serde_json::to_string_pretty(&card).map_err(|e| FcpError::Config(e.to_string()))?;
    fs::write(&catalog_path, body).await.map_err(FcpError::Io)?;
    Ok(card)
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = tags
        .into_iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

async fn load_card_from_path(path: &Path) -> Result<Option<MediaCard>> {
    let raw = match fs::read_to_string(path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(FcpError::Io(e)),
    };
    let card: MediaCard = serde_json::from_str(&raw).map_err(|e| FcpError::ParseFault(e))?;
    Ok(Some(card))
}

pub async fn load_card_by_content_hash(
    workspace_root: &Path,
    content_hash: &str,
) -> Result<Option<MediaCard>> {
    load_card_from_path(&catalog_abs_path(workspace_root, content_hash)).await
}

pub async fn load_card_by_file_path(
    workspace_root: &Path,
    relative_path: &str,
) -> Result<Option<MediaCard>> {
    let hash = content_hash_for_file(workspace_root, relative_path).await?;
    load_card_by_content_hash(workspace_root, &hash).await
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UserNotesPatch {
    #[serde(default)]
    pub set: Option<String>,
    #[serde(default)]
    pub append: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TagsPatch {
    #[serde(default)]
    pub set: Option<Vec<String>>,
    #[serde(default)]
    pub add: Option<Vec<String>>,
    #[serde(default)]
    pub remove: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MediaMetaPatch {
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub user_notes: Option<UserNotesPatch>,
    #[serde(default)]
    pub tags: Option<TagsPatch>,
    #[serde(default)]
    pub type_fields: Option<BTreeMap<String, Value>>,
}

pub async fn apply_meta_patch(
    workspace_root: &Path,
    patch: MediaMetaPatch,
) -> Result<MediaCard> {
    let content_hash = if let Some(h) = patch.content_hash.filter(|s| !s.trim().is_empty()) {
        h
    } else if let Some(ref path) = patch.relative_path {
        content_hash_for_file(workspace_root, path).await?
    } else {
        return Err(FcpError::ToolFault {
            tool_name: "media:meta".into(),
            reason: "relative_path or content_hash required".into(),
        });
    };

    let catalog_path = catalog_abs_path(workspace_root, &content_hash);
    let mut card = load_card_from_path(&catalog_path)
        .await?
        .ok_or_else(|| FcpError::ToolFault {
            tool_name: "media:meta".into(),
            reason: format!("no catalog card for hash {content_hash}"),
        })?;

    if let Some(title) = patch.title {
        card.title = title;
    }
    if let Some(desc) = patch.description {
        card.description = desc;
    }
    if let Some(notes) = patch.user_notes {
        if let Some(set) = notes.set {
            card.user_notes = set;
        } else if let Some(append) = notes.append {
            if !card.user_notes.is_empty() {
                card.user_notes.push('\n');
            }
            card.user_notes.push_str(&append);
        }
    }
    if let Some(tags) = patch.tags {
        if let Some(set) = tags.set {
            card.tags = normalize_tags(set);
        } else {
            if let Some(add) = tags.add {
                card.tags.extend(normalize_tags(add));
            }
            if let Some(remove) = tags.remove {
                let remove_set: std::collections::BTreeSet<String> = remove
                    .into_iter()
                    .map(|t| t.trim().to_lowercase())
                    .collect();
                card.tags.retain(|t| !remove_set.contains(t));
            }
            card.tags = normalize_tags(card.tags.clone());
        }
    }
    if let Some(fields) = patch.type_fields {
        for (k, v) in fields {
            card.type_fields.insert(k, v);
        }
    }
    card.updated_at = unix_now_secs();
    card.source = "media:meta".to_string();

    let body = serde_json::to_string_pretty(&card).map_err(|e| FcpError::Config(e.to_string()))?;
    fs::write(&catalog_path, body).await.map_err(FcpError::Io)?;
    Ok(card)
}

/// Parse JSON for Qdrant ingest (validates required fields).
pub fn parse_media_json(raw: &str) -> Result<MediaCard> {
    let card: MediaCard = serde_json::from_str(raw).map_err(FcpError::ParseFault)?;
    if card.schema_version == 0 || card.title.trim().is_empty() || card.content_hash.is_empty() {
        return Err(FcpError::Config("invalid media.json: missing required fields".into()));
    }
    Ok(card)
}

pub fn card_to_tool_json(card: &MediaCard) -> Value {
    json!({
        "content_hash": card.content_hash,
        "catalog_path": catalog_relative_path(&card.content_hash),
        "file_path": card.file_path,
        "title": card.title,
        "media_type": card.media_type.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::blob_store::sha256_hex;
    use tempfile::tempdir;

    #[test]
    fn infer_media_type_from_path_works() {
        assert_eq!(
            infer_media_type_from_path("99_USER_UPLOADED/images/ab.jpg"),
            Some(MediaType::Image)
        );
    }

    #[test]
    fn build_embed_text_includes_dates() {
        let card = MediaCard {
            schema_version: 1,
            content_hash: "abc".into(),
            media_type: MediaType::Image,
            file_path: "99_USER_UPLOADED/images/x.jpg".into(),
            mime_type: "image/jpeg".into(),
            title: "Truck".into(),
            tags: vec!["food".into()],
            description: "A truck".into(),
            user_notes: "Nice".into(),
            uploaded_at: 1_704_067_200,
            cataloged_at: 1_704_153_600,
            updated_at: 1_704_153_600,
            source: "media:catalog".into(),
            type_fields: BTreeMap::new(),
        };
        let text = build_embed_text(&card);
        assert!(text.contains("Title: Truck"));
        assert!(text.contains("Uploaded:"));
        assert!(text.contains("Description:"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_and_meta_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let img_dir = root.join("99_USER_UPLOADED/images");
        fs::create_dir_all(&img_dir).await.expect("mkdir");
        let bytes = b"jpeg-bytes";
        let hash = sha256_hex(bytes);
        let rel = format!("99_USER_UPLOADED/images/{hash}.jpg");
        fs::write(root.join(&rel), bytes).await.expect("write");

        let card = upsert_catalog(
            root,
            CatalogInput {
                relative_path: rel.clone(),
                title: "Test".into(),
                media_type: Some(MediaType::Image),
                tags: vec!["a".into()],
                description: "desc".into(),
                user_notes: String::new(),
                uploaded_at: Some(100),
                type_fields: BTreeMap::new(),
            },
        )
        .await
        .expect("catalog");

        assert_eq!(card.content_hash, hash);

        let patched = apply_meta_patch(
            root,
            MediaMetaPatch {
                content_hash: Some(hash.clone()),
                relative_path: None,
                title: None,
                description: None,
                user_notes: Some(UserNotesPatch {
                    set: None,
                    append: Some("extra".into()),
                }),
                tags: None,
                type_fields: None,
            },
        )
        .await
        .expect("meta");
        assert!(patched.user_notes.contains("extra"));
    }
}
