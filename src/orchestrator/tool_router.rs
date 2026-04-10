use std::sync::Arc;

use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::Ollama;

use crate::engine::Message;
use crate::executive::error::{FcpError, Result};
use crate::tools::ToolDescriptorRegistry;

pub struct ToolRouter {
    ollama: Arc<Ollama>,
    embed_model: String,
    tool_embeddings: Vec<(String, Vec<f32>)>,
    threshold: f32,
}

impl ToolRouter {
    /// Short greetings and tiny utterances: conversational only (evaluated in orchestrator **before** embedding).
    /// When the stack holds a staged large buffer and the user is clearly continuing that read, keep tools enabled.
    pub fn short_input_guard_conversational_only(text: &str, chat_stack: &[Message]) -> bool {
        if crate::orchestrator::buffer_continuation::stack_has_buffer_routing_context(chat_stack)
            && (Self::has_buffer_continuation_lexical_intent(text)
                || Self::is_short_buffer_followup_ack(text))
        {
            return false;
        }
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

    /// One-line follow-ups after a staged buffer (`ok`, `next`, …) so short-input guard does not drop tool mode.
    pub fn is_short_buffer_followup_ack(text: &str) -> bool {
        let lower = text.trim().to_lowercase();
        matches!(
            lower.as_str(),
            "ok"
                | "okay"
                | "k"
                | "yes"
                | "yep"
                | "yeah"
                | "sure"
                | "next"
                | "more"
                | "mhm"
                | "uh-huh"
                | "uh huh"
        ) || matches!(
            lower.as_str(),
            "go on"
                | "go ahead"
                | "read more"
                | "next page"
                | "keep going"
                | "please continue"
                | "continue"
                | "rescan"
                | "re-scan"
                | "re scan"
        )
    }

    /// User language that suggests paging or continuing a large in-context read (for lexical guard + routing embed tail).
    pub fn has_buffer_continuation_lexical_intent(text: &str) -> bool {
        let lower = text.to_lowercase();
        let phrases = [
            "continue reading",
            "keep reading",
            "read more",
            "next page",
            "next section",
            "following page",
            "rest of the file",
            "rest of the document",
            "more of the file",
            "more of that file",
            "more of the note",
            "more of that note",
            "next chunk",
            "next part",
            "show more",
            "paginate",
            "buffer page",
            "buffer_id",
            "artifact_id",
            "sequential chunks",
            "remaining pages",
            "later section",
            "further down",
        ];
        phrases.iter().any(|p| lower.contains(p))
    }

    fn embed_input_implies_staged_buffer(embed_input: &str) -> bool {
        embed_input.contains("\"buffer_id\"")
            || (embed_input.contains("\"artifact_id\"") && embed_input.contains("chunk_count"))
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
        ollama: Arc<Ollama>,
        embed_model: String,
        tool_descriptions: Vec<(String, String)>,
        descriptors: Option<Arc<ToolDescriptorRegistry>>,
        threshold: f32,
    ) -> Result<Self> {
        let mut tool_embeddings = Vec::with_capacity(tool_descriptions.len());

        for (name, description) in &tool_descriptions {
            let text = Self::enrich_for_routing(name, description, descriptors.as_deref());
            let embedding = Self::embed(&ollama, &embed_model, &text).await?;
            tool_embeddings.push((name.clone(), embedding));
            tracing::debug!(tool = %name, "Pre-computed tool embedding");
        }

        tracing::info!(
            tool_count = tool_embeddings.len(),
            threshold,
            "ToolRouter initialized with pre-computed embeddings"
        );

        Ok(Self { ollama, embed_model, tool_embeddings, threshold })
    }

    fn enrich_for_routing(name: &str, description: &str, descriptors: Option<&ToolDescriptorRegistry>) -> String {
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

    async fn embed(ollama: &Ollama, model: &str, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() {
            return Err(FcpError::EmbeddingFault("Cannot embed empty text".to_string()));
        }
        let request = GenerateEmbeddingsRequest::new(model.to_string(), text.to_string().into());
        let response = ollama
            .generate_embeddings(request)
            .await
            .map_err(|e| FcpError::NetworkFault(e.to_string()))?;
        response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| FcpError::EmbeddingFault("Embedding model returned no vectors".to_string()))
    }

    /// Embed the LLM's thought and compare against all tool embeddings.
    /// Returns tool names whose similarity exceeds the threshold, sorted by
    /// descending similarity.
    /// `embed_input` may include a tail from recent tool output for better similarity to pager tools.
    /// `user_line_for_lexical_guards` is the raw last user message (short-input and buffer guards).
    pub async fn match_tools(
        &self,
        embed_input: &str,
        user_line_for_lexical_guards: &str,
    ) -> Result<Vec<(String, f32)>> {
        if embed_input.trim().is_empty() {
            return Ok(Vec::new());
        }

        let thought_vec = Self::embed(&self.ollama, &self.embed_model, embed_input).await?;

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

        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if hits.is_empty() {
            tracing::debug!(
                thought_preview = &embed_input[..embed_input.len().min(80)],
                "No tool match"
            );
        } else {
            tracing::info!(
                matches = ?hits.iter().map(|(n, s)| format!("{}({:.3})", n, s)).collect::<Vec<_>>(),
                "Semantic tool matches"
            );
        }
        if Self::has_web_lexical_intent(user_line_for_lexical_guards)
            && !hits.iter().any(|(t, _)| t == "web:fetch")
        {
            tracing::info!(
                event = "LEXICAL_TOOL_GUARD",
                forced_tool = "web:fetch",
                thought_preview = %user_line_for_lexical_guards.chars().take(120).collect::<String>(),
                "Forcing web:fetch due to lexical web intent"
            );
            hits.push(("web:fetch".to_string(), 1.0));
        }
        let buffer_user = Self::has_buffer_continuation_lexical_intent(user_line_for_lexical_guards)
            || Self::is_short_buffer_followup_ack(user_line_for_lexical_guards);
        if buffer_user
            && Self::embed_input_implies_staged_buffer(embed_input)
            && !hits.iter().any(|(t, _)| t == "ephemeral:buffer_page")
        {
            tracing::info!(
                event = "LEXICAL_TOOL_GUARD",
                forced_tool = "ephemeral:buffer_page",
                user_preview = %user_line_for_lexical_guards.chars().take(120).collect::<String>(),
                "Forcing ephemeral:buffer_page after staged buffer + continuation phrasing"
            );
            hits.push(("ephemeral:buffer_page".to_string(), 1.0));
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
        assert!(ToolRouter::has_web_lexical_intent("visit https://www.spiegel.de and summarize"));
        assert!(ToolRouter::has_web_lexical_intent("read www.zeit.de news"));
    }

    #[test]
    fn test_web_lexical_intent_with_domain_token() {
        assert!(ToolRouter::has_web_lexical_intent("please open heise.de/newsticker"));
        assert!(!ToolRouter::has_web_lexical_intent("tell me about rust traits"));
    }

    #[test]
    fn test_web_lexical_intent_figurative_open_not_matched() {
        assert!(!ToolRouter::has_web_lexical_intent(
            "you will open the way for future AIs"
        ));
    }

    #[test]
    fn test_web_lexical_intent_visit_page_phrase() {
        assert!(ToolRouter::has_web_lexical_intent("visit the page and summarize"));
        assert!(ToolRouter::has_web_lexical_intent("please open this page"));
    }

    #[test]
    fn test_short_input_guard_without_explicit_intent() {
        assert!(ToolRouter::short_input_guard_conversational_only("test", &[]));
        assert!(!ToolRouter::short_input_guard_conversational_only("https://example.com", &[]));
        assert!(!ToolRouter::short_input_guard_conversational_only("/health", &[]));
    }

    #[test]
    fn short_input_bypass_when_staged_buffer_and_next() {
        use crate::engine::Message;
        use crate::orchestrator::context::format_tool_success_line;

        let stack = vec![Message {
            role: "system".into(),
            content: format_tool_success_line(
                "vault:read",
                "[Large vault file staged as ephemeral buffer]\n\n{\"buffer_id\":\"abc\"}\n",
            ),
        }];
        assert!(!ToolRouter::short_input_guard_conversational_only("next", &stack));
        assert!(!ToolRouter::short_input_guard_conversational_only("ok", &stack));
    }

    #[test]
    fn short_input_bypass_continue_after_buffer_page() {
        use crate::engine::Message;
        use crate::orchestrator::context::format_tool_success_line;

        let body = r#"{"buffer_id":"buf_1","source":"f.md","page":0,"page_size":1,"page_count":2,"total_chunks":2,"next_page":1,"chunks":[]}"#;
        let stack = vec![Message {
            role: "system".into(),
            content: format_tool_success_line("ephemeral:buffer_page", body),
        }];
        assert!(!ToolRouter::short_input_guard_conversational_only("continue", &stack));
        assert!(!ToolRouter::short_input_guard_conversational_only("rescan", &stack));
    }

    #[test]
    fn buffer_continuation_lexical_phrases() {
        assert!(ToolRouter::has_buffer_continuation_lexical_intent(
            "please read more of the file"
        ));
        assert!(ToolRouter::has_buffer_continuation_lexical_intent("next section please"));
    }
}
