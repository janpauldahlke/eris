use std::sync::Arc;
use ollama_rs::Ollama;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
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
        let phrases = [
            "visit ",
            "open ",
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
        let hints = match name {
            "vault:read" => "reading files, checking notes, looking at documents, show me, what is in my vault, review notes, open file, read my notes",
            "vault:write" => "writing files, saving notes, creating documents, write this down, save to vault, take a note, jot down, record",
            "vault:list" => "listing files, what files do I have, show directory, browse vault, what is in my folder, list notes",
            "memory:query" => "remembering, recalling, do you remember, what did I say, past conversations, search memory, who am I, what is my name, user name, my identity, preferences, facts about the user, recall, recognize me, history",
            "memory:commit" => "commit one staged memory by staged_id, persist selected staged entry to long-term memory",
            "memory:commit_all" => "flush all staged memories, persist all staged entries, bulk commit staged memory",
            "memory:staged_list" => "show staged memory ids, list staged entries, what is currently staged before commit",
            "memory:stage" => "stage memory with ttl, temporarily hold fact before explicit commit",
            "agenda:push" => "adding tasks, to-do list, add to agenda queue, schedule, plan, new task without setting a time",
            "agenda:list" => "show tasks, what is on my list, pending items, show agenda, my schedule, what do I have to do",
            "agenda:remove" => "remove task, cancel agenda item, delete from list, drop task, never mind that reminder, scratch that task",
            "agenda:remind_at" => "remind me about this agenda todo, snooze task on my list, alarm linked to agenda task_id, reschedule this queued item, at 3pm for this agenda row",
            "agenda:complete" => "finishing tasks, mark done, complete task, check off, task finished, I did it",
            "web:fetch" => "fetching URLs, web search, look up online, check website, browse internet, search the web, what is happening, news, look this up",
            "web:artifact_query" => "query fetched web artifact by artifact id, search fetched page snippets, retrieve specific sections from buffered webpage",
            "system:health" => "system status, CPU usage, memory usage, disk space, health check, diagnostics, how is the system, performance, resources",
            "clock:now" => "what time is it, current time, timezone, date now, local time",
            "clock:timer" => "generic timer in 30 minutes, countdown, stretch break, ping me in, not tied to agenda list, label-only reminder",
            "clock:alarm" => "wake me at seven, alarm at wall clock time, 7am tomorrow, fixed o'clock time, not an agenda task reminder",
            _ => "",
        };
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
    pub async fn match_tools(&self, thought: &str) -> Result<Vec<(String, f32)>> {
        if thought.trim().is_empty() {
            return Ok(Vec::new());
        }

        let thought_vec = Self::embed(&self.ollama, &self.embed_model, thought).await?;

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
            tracing::debug!(thought_preview = &thought[..thought.len().min(80)], "No tool match");
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
        assert!(ToolRouter::has_web_lexical_intent("visit https://www.spiegel.de and summarize"));
        assert!(ToolRouter::has_web_lexical_intent("read www.zeit.de news"));
    }

    #[test]
    fn test_web_lexical_intent_with_domain_token() {
        assert!(ToolRouter::has_web_lexical_intent("please open heise.de/newsticker"));
        assert!(!ToolRouter::has_web_lexical_intent("tell me about rust traits"));
    }

    #[test]
    fn test_short_input_guard_without_explicit_intent() {
        assert!(ToolRouter::short_input_guard_conversational_only("test"));
        assert!(!ToolRouter::short_input_guard_conversational_only("https://example.com"));
        assert!(!ToolRouter::short_input_guard_conversational_only("/health"));
    }
}
