//! `40_MEDIA` catalog cards — structured descriptors for user-uploaded blobs.

mod card;
mod paths;

pub use card::{
    CatalogInput, MediaCard, MediaMetaPatch, MediaType, TagsPatch, UserNotesPatch,
    apply_meta_patch, build_embed_text, card_to_tool_json, catalog_abs_path,
    catalog_relative_path,
    infer_media_type_from_path, load_card_by_content_hash, load_card_by_file_path,
    media_card_eligible_for_ingest, parse_media_json, upsert_catalog,
};
pub use paths::MEDIA_CATALOG_DIR;
