//! Strip decorative HTML→Markdown noise and prefer **body text after the main headline**.
//! In-body markdown headings are retained but weighted lower for lexical `artifact_query` scoring.

/// Lines that look like markdown section headings (`#` … `######` plus title text).
pub fn line_is_markdown_heading(line: &str) -> bool {
    let t = line.trim_start();
    if !t.starts_with('#') {
        return false;
    }
    let rest = t.trim_start_matches('#').trim_start();
    !rest.is_empty()
}

fn line_is_substantial_paragraph(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.chars().count() >= 48
}

fn line_looks_like_footer(line: &str) -> bool {
    let ll = line.trim().to_lowercase();
    if ll.is_empty() {
        return false;
    }
    const FOOTER: &[&str] = &[
        "impressum",
        "datenschutz",
        "nutzungsbedingungen",
        "all rights reserved",
        "copyright",
        "©",
        "newsletter",
        "abo ",
        "abonnieren",
        "cookie settings",
        "advertisement",
        "sponsored content",
    ];
    FOOTER.iter().any(|kw| ll.contains(kw))
}

fn trim_footer_block(lines: &[&str], start: usize) -> usize {
    let mut end = lines.len();
    let mut i = lines.len();
    while i > start {
        i -= 1;
        let t = lines[i].trim();
        if t.is_empty() {
            continue;
        }
        if line_looks_like_footer(t) {
            end = i;
        } else if t.chars().count() > 48 {
            break;
        }
    }
    end
}

/// Skip lines that add little readable article text (decoration, empty images, tiny nav links).
fn strip_low_value_lines(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in s.lines() {
        let t = line.trim();
        if t.is_empty() {
            out.push(String::new());
            continue;
        }
        if t.starts_with("![") && t.contains("](") {
            continue;
        }
        if t.starts_with("[") {
            if let Some(inner_end) = t.find("](") {
                if inner_end >= 2 {
                    let anchor = &t[1..inner_end];
                    if anchor.trim().chars().count() < 3 {
                        continue;
                    }
                }
            }
        }
        if t.chars()
            .all(|c| c == '-' || c == '*' || c == '_' || c == ' ')
            && t.chars().count() >= 3
        {
            continue;
        }
        out.push(line.to_string());
    }
    collapse_blank_lines(&out.join("\n"), 2)
}

fn collapse_blank_lines(s: &str, max_blank_run: usize) -> String {
    let mut out = Vec::new();
    let mut blanks = 0usize;
    for line in s.lines() {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks <= max_blank_run {
                out.push(String::new());
            }
        } else {
            blanks = 0;
            out.push(line.to_string());
        }
    }
    out.join("\n")
}

/// Start at the first real headline (`#` … `######` with a short title) or first long paragraph.
fn clip_to_primary_article(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut start_idx = None::<usize>;
    for (i, line) in lines.iter().enumerate() {
        if line_is_markdown_heading(line.trim()) {
            start_idx = Some(i);
            break;
        }
    }
    if start_idx.is_none() {
        for (i, line) in lines.iter().enumerate() {
            if line_is_substantial_paragraph(line) {
                start_idx = Some(i);
                break;
            }
        }
    }
    let start = start_idx.unwrap_or(0);

    let end = trim_footer_block(&lines, start);
    let slice: Vec<&str> = lines[start..end].to_vec();
    slice.join("\n")
}

/// Multiplier for lexical ranking: chunks that are mostly `#` headings score lower than prose.
pub fn chunk_heading_weight_factor(chunk: &str) -> f32 {
    let mut non_empty = 0usize;
    let mut headings = 0usize;
    for line in chunk.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        non_empty += 1;
        if line_is_markdown_heading(line) {
            headings += 1;
        }
    }
    if non_empty == 0 {
        return 1.0;
    }
    let ratio = (headings as f32) / (non_empty as f32);
    let factor = 1.0 - 0.45 * ratio.min(1.0);
    factor.max(0.38)
}

/// Focus stored artifact text on article body: headline anchor, trim footer-ish tail, drop junk lines.
pub fn focus_article_text(markdown: &str) -> String {
    let trimmed = markdown.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let clipped = clip_to_primary_article(trimmed);
    let stripped = strip_low_value_lines(&clipped);
    let t = stripped.trim();
    if t.is_empty() {
        strip_low_value_lines(trimmed).trim().to_string()
    } else {
        stripped.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_starts_at_headline() {
        let md = "Home\nShop\n\n# Real headline\n\nBody paragraph one with enough characters to be readable text here.\n\nMore body.";
        let out = focus_article_text(md);
        assert!(out.contains("Real headline"));
        assert!(!out.contains("Home"));
        assert!(out.contains("Body paragraph"));
    }

    #[test]
    fn falls_back_to_long_line_when_no_hash_heading() {
        let md = "x\ny\n\nThis is a long enough line of plain text without hash headings to trigger substantial paragraph logic.";
        let out = focus_article_text(md);
        assert!(out.contains("long enough line"));
        assert!(!out.starts_with("x\n"));
    }

    #[test]
    fn heading_weight_lowers_mostly_heading_chunk() {
        let prose = "economy markets update with detail";
        let heads = "## A\n## B\n## C\n## D\n";
        let wp = chunk_heading_weight_factor(prose);
        let wh = chunk_heading_weight_factor(heads);
        assert!(wp > wh);
    }
}
