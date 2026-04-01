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
}
