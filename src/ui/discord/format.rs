//! Split assistant text into Discord-safe chunks (content length limit).

/// Discord message `content` must be at most 2000 UTF-16 code units; we use a conservative char budget.
pub fn chunk_discord_content(content: &str, max_chars: usize) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    let max_chars = max_chars.max(1);
    let mut out = Vec::new();
    let mut rest = content;
    while !rest.is_empty() {
        if rest.chars().count() <= max_chars {
            out.push(rest.to_string());
            break;
        }
        let mut end = 0usize;
        let mut n = 0usize;
        for (i, ch) in rest.char_indices() {
            if n >= max_chars {
                break;
            }
            n = n.saturating_add(1);
            end = i + ch.len_utf8();
        }
        if end == 0 {
            // Extremely pathological `max_chars` / first char; take one codepoint.
            let ch = rest.chars().next().unwrap_or_default();
            end = ch.len_utf8().max(1);
        }
        out.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_empty() {
        assert!(chunk_discord_content("", 10).is_empty());
    }

    #[test]
    fn chunk_splits_long_ascii() {
        let s = "a".repeat(50);
        let parts = chunk_discord_content(&s, 20);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts.iter().map(|p| p.len()).sum::<usize>(), 50);
    }
}
