//! End-of-turn side effects for ephemeral memory (Zettel Phase 2): user-text mention matching
//! and prompt digest formatting. Vault commit remains user-initiated only.

use unicode_normalization::UnicodeNormalization;

use crate::config::AppConfig;
use crate::memory::ephemeral::{EphemeralMemory, is_web_artifact_staging};
use crate::memory::types::EphemeralTier;
use crate::orchestrator::context::ROLLING_SUMMARY_TITLE;

/// Outcome of applying user-turn mention boosts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TurnEndMentionStats {
    pub entries_matched: usize,
}

fn normalize_user_words(text: &str) -> std::collections::HashSet<String> {
    let nfkc: String = text.nfkc().collect::<String>().to_lowercase();
    let mut words = std::collections::HashSet::new();
    let mut cur = String::new();
    for ch in nfkc.chars() {
        if ch.is_alphanumeric() {
            cur.push(ch);
        } else if !cur.is_empty() {
            if cur.len() >= 3 {
                words.insert(cur.clone());
            }
            cur.clear();
        }
    }
    if cur.len() >= 3 {
        words.insert(cur);
    }
    words
}

fn should_skip_entry(title: &str, tags: &[String], canonical_key: &str) -> bool {
    if title == ROLLING_SUMMARY_TITLE {
        return true;
    }
    if title.starts_with("fcp:") || canonical_key.starts_with("fcp:") {
        return true;
    }
    if is_web_artifact_staging(tags, title) {
        return true;
    }
    false
}

/// True if user message words match this row strongly enough (deterministic, no embeddings).
fn user_covers_canonical_key(
    user_words: &std::collections::HashSet<String>,
    canonical_key: &str,
) -> bool {
    let tokens: Vec<&str> = canonical_key.split('_').filter(|t| t.len() >= 3).collect();
    if tokens.is_empty() {
        return false;
    }
    let mut hits = 0usize;
    let mut longest_hit = 0usize;
    for t in &tokens {
        if user_words.contains(*t) {
            hits += 1;
            longest_hit = longest_hit.max(t.len());
        }
    }
    if hits >= 2 {
        return true;
    }
    hits >= 1 && longest_hit >= 5
}

/// After a full user turn completes: bump score, mention_count, refresh TTL for matching staged rows.
pub async fn apply_user_turn_mentions(
    memory: &EphemeralMemory,
    user_text: &str,
    config: &AppConfig,
) -> TurnEndMentionStats {
    if !config.turn_end_mention_enabled {
        return TurnEndMentionStats::default();
    }
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return TurnEndMentionStats::default();
    }

    let user_words = normalize_user_words(trimmed);
    if user_words.is_empty() {
        return TurnEndMentionStats::default();
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut matched = 0usize;
    let entries = memory.list_entries();

    for entry in entries {
        if should_skip_entry(&entry.title, &entry.tags, &entry.canonical_key) {
            continue;
        }
        if !user_covers_canonical_key(&user_words, &entry.canonical_key) {
            continue;
        }

        let ttl = config.ttl_for_tier(entry.tier);
        let mut updated = entry.clone();
        updated.promotion_score += config.promotion_mention_boost;
        updated.mention_count = updated.mention_count.saturating_add(1);
        updated.last_seen_at = now;
        updated.expires_at = now.saturating_add(ttl);

        memory
            .cache
            .insert(updated.staged_id.clone(), updated)
            .await;
        matched += 1;
    }

    if matched > 0 {
        tracing::info!(
            entries_matched = matched,
            boost = config.promotion_mention_boost,
            "Turn-end mention hook: refreshed matching staged memories"
        );
    }

    TurnEndMentionStats {
        entries_matched: matched,
    }
}

fn tier_rank(t: EphemeralTier) -> u8 {
    match t {
        EphemeralTier::Promote => 0,
        EphemeralTier::Scratch => 1,
        EphemeralTier::Session => 2,
    }
}

/// Compact sidebar for system prompts so the model sees staged state without calling tools.
pub fn format_staged_digest_for_prompt(ephemeral: &EphemeralMemory, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let mut rows: Vec<_> = ephemeral
        .list_entries()
        .into_iter()
        .filter(|e| !should_skip_entry(&e.title, &e.tags, &e.canonical_key))
        .collect();

    if rows.is_empty() {
        return String::new();
    }

    rows.sort_by(|a, b| {
        tier_rank(a.tier)
            .cmp(&tier_rank(b.tier))
            .then_with(|| a.title.cmp(&b.title))
    });

    let header = "[ACTIVE_STAGED_MEMORY]\n\
        Runtime-maintained: scores and TTL refresh when the user's message matches a row's topic (canonical key). \
        The user alone persists to disk via memory:commit / memory:commit_all (promote tier only for bulk).\n";
    let footer = "[/ACTIVE_STAGED_MEMORY]\n";

    let mut out = String::new();
    out.push_str(header);
    let mut wrote_any = false;

    for e in rows {
        let preview: String = e.data.chars().take(100).collect();
        let line = format!(
            "- [{}] score={:.1} tier={} | {} — {}\n",
            e.staged_id.chars().take(8).collect::<String>(),
            e.promotion_score,
            e.tier,
            e.title,
            preview.trim_end()
        );
        if out.len() + line.len() + footer.len() > max_chars {
            break;
        }
        out.push_str(&line);
        wrote_any = true;
    }

    if !wrote_any {
        return String::new();
    }

    out.push_str(footer);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{EphemeralTier, VaultKind};

    #[tokio::test]
    async fn mention_hook_bumps_score_and_refreshes_ttl() {
        let memory = EphemeralMemory::new("t".into());
        let mut cfg = AppConfig::default();
        cfg.turn_end_mention_enabled = true;
        cfg.promotion_mention_boost = 1.25;
        cfg.ephemeral_ttl_session_secs = 999;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let entry = crate::memory::ephemeral::CacheValue {
            staged_id: "sid-1".into(),
            title: "hagbard_loves_piano".into(),
            data: "likes jazz".into(),
            tags: vec!["user".into()],
            expires_at: now + 10,
            node_id: "nid".into(),
            canonical_key: "hagbard_loves_piano".into(),
            tier: EphemeralTier::Session,
            promotion_score: 2.0,
            mention_count: 1,
            needs_review: false,
            first_seen_at: now,
            last_seen_at: now,
            kind: VaultKind::Synthesis,
        };
        memory.cache.insert("sid-1".into(), entry).await;

        let stats = apply_user_turn_mentions(&memory, "I still love piano and hagbard", &cfg).await;
        assert_eq!(stats.entries_matched, 1);

        let updated = memory.get_by_id("sid-1").await.expect("row");
        assert!((updated.promotion_score - 3.25).abs() < 0.001);
        assert_eq!(updated.mention_count, 2);
        assert!(updated.expires_at >= now + 900);
    }

    #[tokio::test]
    async fn mention_hook_skips_when_disabled() {
        let memory = EphemeralMemory::new("t".into());
        let mut cfg = AppConfig::default();
        cfg.turn_end_mention_enabled = false;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let entry = crate::memory::ephemeral::CacheValue {
            staged_id: "sid-2".into(),
            title: "topic_alpha_beta".into(),
            data: "x".into(),
            tags: vec!["t".into()],
            expires_at: now + 500,
            node_id: "nid2".into(),
            canonical_key: "topic_alpha_beta".into(),
            tier: EphemeralTier::Session,
            promotion_score: 1.0,
            mention_count: 1,
            needs_review: false,
            first_seen_at: now,
            last_seen_at: now,
            kind: VaultKind::Synthesis,
        };
        memory.cache.insert("sid-2".into(), entry).await;

        let stats = apply_user_turn_mentions(&memory, "alpha beta topic here", &cfg).await;
        assert_eq!(stats.entries_matched, 0);
        let u = memory.get_by_id("sid-2").await.expect("row");
        assert_eq!(u.promotion_score, 1.0);
    }
}
