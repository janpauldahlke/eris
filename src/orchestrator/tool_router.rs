use std::sync::Arc;
use ollama_rs::Ollama;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use crate::executive::error::{FcpError, Result};

pub struct ToolRouter {
    ollama: Arc<Ollama>,
    embed_model: String,
    tool_embeddings: Vec<(String, Vec<f32>)>,
    threshold: f32,
}

impl ToolRouter {
    pub async fn new(
        ollama: Arc<Ollama>,
        embed_model: String,
        tool_descriptions: Vec<(String, String)>,
        threshold: f32,
    ) -> Result<Self> {
        let mut tool_embeddings = Vec::with_capacity(tool_descriptions.len());

        for (name, description) in &tool_descriptions {
            let text = Self::enrich_for_routing(&name, &description);
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

    fn enrich_for_routing(name: &str, description: &str) -> String {
        let hints = match name {
            "vault:read" => "reading files, checking notes, looking at documents, show me, what is in my vault, review notes, open file, read my notes",
            "vault:write" => "writing files, saving notes, creating documents, write this down, save to vault, take a note, jot down, record",
            "vault:list" => "listing files, what files do I have, show directory, browse vault, what is in my folder, list notes",
            "memory:query" => "remembering, recalling, do you remember, what did I say, past conversations, search memory, who am I, what is my name, recall, recognize me, history",
            "memory:commit" => "commit one staged memory by staged_id, persist selected staged entry to long-term memory",
            "memory:commit_all" => "flush all staged memories, persist all staged entries, bulk commit staged memory",
            "memory:staged_list" => "show staged memory ids, list staged entries, what is currently staged before commit",
            "memory:stage" => "stage memory with ttl, temporarily hold fact before explicit commit",
            "agenda:push" => "adding tasks, to-do list, remind me to, schedule, plan, add to agenda, new task, I need to",
            "agenda:list" => "show tasks, what is on my list, pending items, show agenda, my schedule, what do I have to do",
            "agenda:complete" => "finishing tasks, mark done, complete task, check off, task finished, I did it",
            "web:fetch" => "fetching URLs, web search, look up online, check website, browse internet, search the web, what is happening, news, look this up",
            "system:health" => "system status, CPU usage, memory usage, disk space, health check, diagnostics, how is the system, performance, resources",
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
    pub async fn match_tools(&self, thought: &str) -> Result<Vec<String>> {
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

        Ok(hits.into_iter().map(|(name, _)| name).collect())
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
}
