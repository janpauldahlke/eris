use crate::engine::EmbeddingProvider;
use crate::executive::error::{FcpError, Result};
use crate::tools::ToolDescriptorRegistry;
use std::sync::Arc;

/// Cosine similarity floor for `moltbook:*` tools when the user line has no explicit Moltbook wording.
///
/// General chat often lands near ~0.50–0.56 against notification/comment/search embeddings; memory
/// recall phrases are particularly close. Real Moltbook requests typically score clearly above this.
const MOLTBOOK_SEMANTIC_MIN: f32 = 0.58;

fn has_moltbook_lexical_intent(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("moltbook") || lower.contains("submolt")
}

fn filter_moltbook_semantic_hits(thought: &str, hits: Vec<(String, f32)>) -> Vec<(String, f32)> {
    let lexical = has_moltbook_lexical_intent(thought);
    if lexical {
        return hits;
    }
    hits.into_iter()
        .filter(|(name, sim)| {
            if name.starts_with("moltbook:") && *sim < MOLTBOOK_SEMANTIC_MIN {
                tracing::debug!(
                    tool = %name,
                    similarity = sim,
                    min_without_lexical = MOLTBOOK_SEMANTIC_MIN,
                    "Moltbook tool match dropped (raise similarity or mention Moltbook explicitly)"
                );
                false
            } else {
                true
            }
        })
        .collect()
}

pub struct ToolRouter {
    embed: Arc<dyn EmbeddingProvider>,
    tool_embeddings: Vec<(String, Vec<f32>)>,
    threshold: f32,
}

impl ToolRouter {
    /// Short greetings and tiny utterances: conversational only (evaluated in orchestrator **before** embedding).
    pub fn short_input_guard_conversational_only(text: &str) -> bool {
        Self::is_short_input_without_explicit_tool_intent(text)
    }

    fn is_short_input_without_explicit_tool_intent(text: &str) -> bool {
        let trimmed = text.trim();
        let token_count = trimmed.split_whitespace().count();
        let is_short = token_count <= 3 || trimmed.chars().count() <= 15;
        if !is_short {
            return false;
        }
        let lower = trimmed.to_lowercase();
        let explicit = lower.starts_with('/')
            || lower.contains("http://")
            || lower.contains("https://")
            || lower.contains("www.")
            || Self::has_domain_like_token(&lower)
            || lower.contains("search web for")
            || lower.contains("search the web")
            || lower.contains("look up online");
        !explicit
    }

    fn has_domain_like_token(text: &str) -> bool {
        text.split_whitespace().any(|raw| {
            let token = raw
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-' && c != '/')
                .to_lowercase();
            token.contains('.')
                && !token.starts_with('.')
                && !token.ends_with('.')
                && (token.ends_with(".de")
                    || token.ends_with(".com")
                    || token.ends_with(".org")
                    || token.ends_with(".net")
                    || token.ends_with(".io")
                    || token.contains(".de/")
                    || token.contains(".com/")
                    || token.contains(".org/")
                    || token.contains(".net/")
                    || token.contains(".io/"))
        })
    }

    fn has_web_lexical_intent(text: &str) -> bool {
        let lower = text.to_lowercase();
        if lower.contains("http://") || lower.contains("https://") || lower.contains("www.") {
            return true;
        }
        if Self::has_domain_like_token(&lower) {
            return true;
        }
        // Multi-word phrases only: bare "open " / "visit " matched figurative English ("open the way").
        let phrases = [
            "visit page",
            "visit the page",
            "visit this page",
            "visit that page",
            "visit a page",
            "visit site",
            "visit the site",
            "open page",
            "open the page",
            "open this page",
            "open that page",
            "open url",
            "open link",
            "open website",
            "open the website",
            "read website",
            "read the website",
            "news from",
            "check website",
            "look up online",
            "search the web",
        ];
        phrases.iter().any(|p| lower.contains(p))
    }

    pub async fn new(
        embed: Arc<dyn EmbeddingProvider>,
        tool_descriptions: Vec<(String, String)>,
        descriptors: Option<Arc<ToolDescriptorRegistry>>,
        threshold: f32,
    ) -> Result<Self> {
        let mut tool_embeddings = Vec::with_capacity(tool_descriptions.len());

        for (name, description) in &tool_descriptions {
            let text = Self::enrich_for_routing(name, description, descriptors.as_deref());
            let embedding = embed.embed(&text).await?;
            tool_embeddings.push((name.clone(), embedding));
            tracing::debug!(tool = %name, "Pre-computed tool embedding");
        }

        tracing::info!(
            tool_count = tool_embeddings.len(),
            threshold,
            "ToolRouter initialized with pre-computed embeddings"
        );

        Ok(Self {
            embed,
            tool_embeddings,
            threshold,
        })
    }

    fn enrich_for_routing(
        name: &str,
        description: &str,
        descriptors: Option<&ToolDescriptorRegistry>,
    ) -> String {
        if let Some(registry) = descriptors
            && let Some(desc) = registry.get(name)
            && !desc.routing_hints.is_empty()
        {
            return format!(
                "{}: {}. Common triggers: {}",
                name,
                description,
                desc.routing_hints.join(", ")
            );
        }
        let hints = crate::tools::routing_phrases::fallback_triggers(name);
        if hints.is_empty() {
            format!("{}: {}", name, description)
        } else {
            format!("{}: {}. Common triggers: {}", name, description, hints)
        }
    }

    async fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() {
            return Err(FcpError::EmbeddingFault(
                "Cannot embed empty text".to_string(),
            ));
        }
        self.embed.embed(text).await
    }

    /// Embed the LLM's thought and compare against all tool embeddings.
    /// Returns tool names whose similarity exceeds the threshold, sorted by
    /// descending similarity.
    pub async fn match_tools(&self, thought: &str) -> Result<Vec<(String, f32)>> {
        if thought.trim().is_empty() {
            return Ok(Vec::new());
        }

        let thought_vec = self.embed_text(thought).await?;

        let mut hits: Vec<(String, f32)> = self
            .tool_embeddings
            .iter()
            .filter_map(|(name, emb)| {
                let sim = cosine_similarity(&thought_vec, emb);
                tracing::trace!(tool = %name, similarity = sim, "Tool similarity");
                if sim >= self.threshold {
                    Some((name.clone(), sim))
                } else {
                    None
                }
            })
            .collect();

        hits = filter_moltbook_semantic_hits(thought, hits);

        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if hits.is_empty() {
            tracing::debug!(
                thought_preview = &thought[..thought.len().min(80)],
                "No tool match"
            );
        } else {
            tracing::info!(
                matches = ?hits.iter().map(|(n, s)| format!("{}({:.3})", n, s)).collect::<Vec<_>>(),
                "Semantic tool matches"
            );
        }
        if Self::has_web_lexical_intent(thought) && !hits.iter().any(|(t, _)| t == "web:fetch") {
            tracing::info!(
                event = "LEXICAL_TOOL_GUARD",
                forced_tool = "web:fetch",
                thought_preview = %thought.chars().take(120).collect::<String>(),
                "Forcing web:fetch due to lexical web intent"
            );
            hits.push(("web:fetch".to_string(), 1.0));
        }
        Ok(hits)
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_identical() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_zero_vector() {
        let z = vec![0.0, 0.0, 0.0];
        let v = vec![1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&z, &v), 0.0);
    }

    #[test]
    fn test_cosine_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_web_lexical_intent_with_url() {
        assert!(ToolRouter::has_web_lexical_intent(
            "visit https://www.spiegel.de and summarize"
        ));
        assert!(ToolRouter::has_web_lexical_intent("read www.zeit.de news"));
    }

    #[test]
    fn test_web_lexical_intent_with_domain_token() {
        assert!(ToolRouter::has_web_lexical_intent(
            "please open heise.de/newsticker"
        ));
        assert!(!ToolRouter::has_web_lexical_intent(
            "tell me about rust traits"
        ));
    }

    #[test]
    fn test_web_lexical_intent_figurative_open_not_matched() {
        assert!(!ToolRouter::has_web_lexical_intent(
            "you will open the way for future AIs"
        ));
    }

    #[test]
    fn test_web_lexical_intent_visit_page_phrase() {
        assert!(ToolRouter::has_web_lexical_intent(
            "visit the page and summarize"
        ));
        assert!(ToolRouter::has_web_lexical_intent("please open this page"));
    }

    #[test]
    fn test_short_input_guard_without_explicit_intent() {
        assert!(ToolRouter::short_input_guard_conversational_only("test"));
        assert!(!ToolRouter::short_input_guard_conversational_only(
            "https://example.com"
        ));
        assert!(!ToolRouter::short_input_guard_conversational_only(
            "/health"
        ));
    }

    #[test]
    fn test_moltbook_lexical_intent() {
        assert!(has_moltbook_lexical_intent("check Moltbook for replies"));
        assert!(has_moltbook_lexical_intent(
            "read the rust submolt feed"
        ));
        assert!(!has_moltbook_lexical_intent(
            "what do you remember from last time"
        ));
    }

    #[test]
    fn test_filter_moltbook_semantic_hits_drops_weak_without_lexical() {
        let hits = vec![
            ("memory:query".to_string(), 0.62f32),
            ("moltbook:notifications_read".to_string(), 0.505f32),
        ];
        let out = filter_moltbook_semantic_hits("what did we talk about yesterday", hits);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "memory:query");
    }

    #[test]
    fn test_filter_moltbook_semantic_hits_keeps_weak_with_lexical() {
        let hits = vec![("moltbook:notifications_read".to_string(), 0.505f32)];
        let out = filter_moltbook_semantic_hits("clear moltbook notifications", hits);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "moltbook:notifications_read");
    }
}
