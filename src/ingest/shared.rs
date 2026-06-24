pub fn truncate_char_boundary(input: &str, max_len: usize) -> String {
    if input.len() <= max_len {
        return input.to_string();
    }
    let mut limit = max_len;
    while limit > 0 && !input.is_char_boundary(limit) {
        limit -= 1;
    }
    input[..limit].to_string()
}

/// Paragraph-aware chunking with overlap for large document RAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkConfig {
    pub target_chars: usize,
    /// Hard ceiling per chunk (re-split after paragraph merge).
    pub max_chars: usize,
    pub overlap_chars: usize,
    pub min_chunk_chars: usize,
}

fn overlap_tail(input: &str, overlap_chars: usize) -> String {
    if overlap_chars == 0 || input.is_empty() {
        return String::new();
    }
    if input.len() <= overlap_chars {
        return input.to_string();
    }
    let start = input.len().saturating_sub(overlap_chars);
    let mut begin = start;
    while begin < input.len() && !input.is_char_boundary(begin) {
        begin += 1;
    }
    input[begin..].to_string()
}

fn split_long_paragraph(paragraph: &str, target_chars: usize, overlap_chars: usize) -> Vec<String> {
    if paragraph.is_empty() {
        return Vec::new();
    }
    let target = target_chars.max(1);
    let overlap = overlap_chars.min(target.saturating_sub(1));
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < paragraph.len() {
        let mut end = (start + target).min(paragraph.len());
        while end > start && !paragraph.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            break;
        }
        out.push(paragraph[start..end].to_string());
        if end >= paragraph.len() {
            break;
        }
        let emitted_len = end - start;
        let rewind = overlap.min(emitted_len);
        let next = end - rewind;
        start = next;
        while start < paragraph.len() && !paragraph.is_char_boundary(start) {
            start += 1;
        }
    }
    out
}

fn enforce_chunk_max_chars(
    chunks: Vec<String>,
    max_chars: usize,
    overlap_chars: usize,
) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let overlap = overlap_chars.min(max_chars.saturating_sub(1));
    let mut out = Vec::new();
    for chunk in chunks {
        if chunk.len() <= max_chars {
            out.push(chunk);
            continue;
        }
        out.extend(split_long_paragraph(&chunk, max_chars, overlap));
    }
    out
}

/// Split document text into overlapping chunks on paragraph boundaries when possible.
pub fn chunk_document(text: &str, cfg: &ChunkConfig) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let target = cfg.target_chars.max(1);
    let max_chars = cfg.max_chars.max(target);
    let overlap = cfg.overlap_chars.min(target.saturating_sub(1));
    let min_chunk = cfg.min_chunk_chars;

    let paragraphs: Vec<&str> = trimmed
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if paragraphs.is_empty() {
        return Vec::new();
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut para_idx = 0usize;

    while para_idx < paragraphs.len() {
        let para = paragraphs[para_idx];
        if para.len() > target {
            if !buf.trim().is_empty() {
                chunks.push(buf.trim().to_string());
                buf = overlap_tail(chunks.last().map(String::as_str).unwrap_or(""), overlap);
            }
            let parts = split_long_paragraph(para, target, overlap);
            if parts.is_empty() {
                para_idx += 1;
                continue;
            }
            if !buf.trim().is_empty() {
                let first = &parts[0];
                let join = format!("{}\n\n{}", buf.trim(), first);
                if join.len() <= target {
                    chunks.push(join);
                    chunks.extend(parts.into_iter().skip(1));
                } else {
                    chunks.push(buf.trim().to_string());
                    chunks.extend(parts);
                }
            } else {
                chunks.extend(parts);
            }
            buf = overlap_tail(chunks.last().map(String::as_str).unwrap_or(""), overlap);
            para_idx += 1;
            continue;
        }

        let addition = if buf.is_empty() {
            para.to_string()
        } else {
            format!("\n\n{para}")
        };
        if !buf.is_empty() && buf.len() + addition.len() > target {
            chunks.push(buf.trim().to_string());
            buf = overlap_tail(chunks.last().map(String::as_str).unwrap_or(""), overlap);
            // If the paragraph STILL won't fit after flushing (overlap + para > target),
            // drop the overlap to prevent an infinite loop where para_idx never advances.
            let retry_len = if buf.is_empty() {
                para.len()
            } else {
                buf.len() + 2 + para.len()
            };
            if retry_len > target {
                buf.clear();
            }
            continue;
        }
        buf.push_str(&addition);
        para_idx += 1;
    }

    if !buf.trim().is_empty() {
        chunks.push(buf.trim().to_string());
    }

    if chunks.len() >= 2 {
        let last_len = chunks.last().map(|c| c.len()).unwrap_or(0);
        if last_len < min_chunk {
            let tail = chunks.pop().unwrap_or_default();
            if let Some(prev) = chunks.last_mut() {
                let merged_len = prev.len().saturating_add(2).saturating_add(tail.len());
                if merged_len <= max_chars {
                    if !prev.is_empty() {
                        prev.push_str("\n\n");
                    }
                    prev.push_str(&tail);
                } else {
                    chunks.push(tail);
                }
            } else {
                chunks.push(tail);
            }
        }
    }

    enforce_chunk_max_chars(chunks, max_chars, overlap)
}

pub fn split_into_chunks(input: &str, chunk_chars: usize) -> Vec<String> {
    if input.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < input.len() {
        let mut end = (start + chunk_chars).min(input.len());
        while end > start && !input.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            break;
        }
        out.push(input[start..end].to_string());
        start = end;
    }
    out
}

pub fn trim_chars(input: &str, max_len: usize) -> String {
    truncate_char_boundary(input, max_len)
}

pub fn trim_snippets_to_budget(snippets: &mut [String], max_total_chars: usize) {
    let mut used = 0usize;
    for snippet in snippets {
        if used >= max_total_chars {
            snippet.clear();
            continue;
        }
        let remaining = max_total_chars - used;
        let trimmed = trim_chars(snippet, remaining);
        used += trimmed.len();
        *snippet = trimmed;
    }
}

pub fn bound_chunks_and_preview(
    input: &str,
    max_bytes: usize,
    chunk_chars: usize,
    preview_chars: usize,
) -> (Vec<String>, String) {
    let bounded = truncate_char_boundary(input, max_bytes);
    let chunks = split_into_chunks(&bounded, chunk_chars.max(256));
    let preview_head = chunks
        .first()
        .map(|c| truncate_char_boundary(c, preview_chars.max(128)))
        .unwrap_or_default();
    (chunks, preview_head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_keeps_utf8_boundaries() {
        let input = "GrüßeausBerlin";
        let chunks = split_into_chunks(input, 5);
        assert!(!chunks.is_empty());
        assert_eq!(chunks.join(""), input);
    }

    #[test]
    fn test_bound_chunks_and_preview_parity() {
        let input = "A".repeat(1000);
        let (chunks, preview) = bound_chunks_and_preview(&input, 50, 32, 32);
        assert!(!chunks.is_empty());
        assert_eq!(preview.len(), 50);
    }

    #[test]
    fn test_budget_trim_clears_overflow_snippets() {
        let mut snippets = vec![
            "12345".to_string(),
            "67890".to_string(),
            "abcde".to_string(),
        ];
        trim_snippets_to_budget(&mut snippets, 8);
        assert_eq!(snippets[0], "12345");
        assert_eq!(snippets[1], "678");
        assert!(snippets[2].is_empty());
    }

    #[test]
    fn chunk_document_respects_paragraphs() {
        let text = "Para one.\n\nPara two.\n\nPara three.";
        let cfg = ChunkConfig {
            target_chars: 20,
            max_chars: 20,
            overlap_chars: 5,
            min_chunk_chars: 5,
        };
        let chunks = chunk_document(text, &cfg);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(!chunk.is_empty());
        }
    }

    #[test]
    fn chunk_document_merges_tiny_tail() {
        let text = format!("{}\n\n{}", "A".repeat(50), "tiny");
        let cfg = ChunkConfig {
            target_chars: 60,
            max_chars: 60,
            overlap_chars: 10,
            min_chunk_chars: 20,
        };
        let chunks = chunk_document(&text, &cfg);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("tiny"));
    }

    #[test]
    fn chunk_document_utf8_safe_overlap() {
        let text = format!("{}\n\n{}", "Grüße".repeat(30), "Ende.");
        let cfg = ChunkConfig {
            target_chars: 80,
            max_chars: 80,
            overlap_chars: 15,
            min_chunk_chars: 10,
        };
        let chunks = chunk_document(&text, &cfg);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(std::str::from_utf8(chunk.as_bytes()).is_ok());
        }
    }

    #[test]
    fn chunk_document_empty_input() {
        let cfg = ChunkConfig {
            target_chars: 100,
            max_chars: 100,
            overlap_chars: 10,
            min_chunk_chars: 10,
        };
        assert!(chunk_document("   ", &cfg).is_empty());
    }

    #[test]
    fn chunk_document_no_infinite_loop_on_overlap_sized_paragraph() {
        // Reproduces the freeze bug: a paragraph of 420 chars with target=480 and
        // overlap=80 means the overlap tail (80) + "\n\n" (2) + para (420) = 502 > 480,
        // so the flush branch fires every iteration without advancing para_idx.
        let para_a = "A".repeat(420);
        let para_b = "B".repeat(420);
        let text = format!("{para_a}\n\n{para_b}");
        let cfg = ChunkConfig {
            target_chars: 480,
            max_chars: 480,
            overlap_chars: 80,
            min_chunk_chars: 100,
        };
        let chunks = chunk_document(&text, &cfg);
        assert!(
            chunks.len() < 20,
            "expected a handful of chunks, got {} — likely infinite loop",
            chunks.len()
        );
        let all_text: String = chunks.join("");
        assert!(all_text.contains("AAAA"));
        assert!(all_text.contains("BBBB"));
    }

    #[test]
    fn chunk_document_never_exceeds_max_chars() {
        let citations = "Villanueva, Roger; Perricone, Valentina; Fiorito, Graziano (2017). ";
        let text = citations.repeat(40);
        let cfg = ChunkConfig {
            target_chars: 480,
            max_chars: 480,
            overlap_chars: 80,
            min_chunk_chars: 100,
        };
        let chunks = chunk_document(&text, &cfg);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(
                chunk.len() <= 480,
                "chunk len {} exceeds max 480",
                chunk.len()
            );
        }
    }
}
