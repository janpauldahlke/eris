//! Generic outbound link ranking for fetched HTML: prefer page-like targets, drop obvious image/asset URLs.
//! Heuristics are site-agnostic (extensions, path shape, anchor text), not tuned to any single publisher.
//!
//! **Relative `href`s** (e.g. `/politik/article`, Heise/TAZ-style) are resolved with [`Url::join`] against the
//! fetched page URL. No cookies are required for this step: it runs on the HTML body already returned by
//! `web:fetch`. Cookie / bot challenges affect **subsequent** requests to follow a discovered URL, not
//! parsing links out of the first response.

use crate::tools::web::artifact::WebOutboundLink;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashSet;
use std::sync::OnceLock;
use url::Url;

/// Stored on [`super::artifact::WebArtifact`] and echoed in the fetch receipt (bounded).
pub const OUTBOUND_LINK_CAP: usize = 24;

static IMG_EXT: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "svg", "ico", "avif", "bmp", "jfif",
];

/// Filenames like `photo-1200x800.jpg` or `thumb_300x200.png` (common on CDNs worldwide).
static DIMENSION_FILENAME: OnceLock<Option<Regex>> = OnceLock::new();

fn dimension_filename_re() -> Option<&'static Regex> {
    DIMENSION_FILENAME
        .get_or_init(|| Regex::new(r"(?i)-\d{2,5}[x×]\d{2,5}\.[a-z0-9]{2,8}$").ok())
        .as_ref()
}

fn path_has_image_extension(path: &str) -> bool {
    let lower = path.to_lowercase();
    let path_only = lower.split('?').next().unwrap_or("");
    let Some(dot) = path_only.rfind('.') else {
        return false;
    };
    let ext = &path_only[dot + 1..];
    IMG_EXT.contains(&ext)
}

fn path_has_dimension_image_filename(path: &str) -> bool {
    let seg = path.split('/').next_back().unwrap_or("");
    dimension_filename_re().is_some_and(|re| re.is_match(seg))
}

fn skip_scheme_or_void(url: &Url) -> bool {
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return true;
    }
    let Some(host) = url.host_str() else {
        return true;
    };
    host.is_empty()
}

fn anchor_collapsed_text(element: ElementRef<'_>) -> String {
    element
        .text()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Visible text, with `title` / `aria-label` when the node has little or no readable inner text (common on
/// image-heavy teasers or icon-only controls).
fn anchor_text_for_scoring(element: ElementRef<'_>) -> String {
    let collapsed = anchor_collapsed_text(element);
    if collapsed.chars().count() >= 12 {
        return collapsed;
    }
    let fallback = element
        .value()
        .attr("title")
        .or_else(|| element.value().attr("aria-label"))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match fallback {
        Some(t) if collapsed.is_empty() || t.chars().count() >= collapsed.chars().count() => {
            t.to_string()
        }
        _ => collapsed,
    }
}

fn resolve_anchor(base: &Url, href: &str) -> Option<Url> {
    let h = href.trim();
    if h.is_empty() || h.starts_with('#') {
        return None;
    }
    let Ok(joined) = base.join(h) else {
        return None;
    };
    if joined.fragment().is_some() && joined.path() == base.path() && joined.query() == base.query() {
        return None;
    }
    Some(joined)
}

fn img_only_anchor(img_sel: &Selector, element: ElementRef<'_>, anchor_text: &str) -> bool {
    element.select(img_sel).next().is_some() && anchor_text.chars().count() < 3
}

fn score_candidate(
    resolved: &Url,
    anchor_text: &str,
    base: &Url,
    img_only_low_text: bool,
) -> Option<i32> {
    let path = resolved.path();

    if path_has_image_extension(path) || path_has_dimension_image_filename(path) {
        return None;
    }

    let mut score: i32 = 0;

    let segments = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .count();
    score += (segments.min(12) as i32) * 3;

    let alen = anchor_text.chars().count().min(160);
    score += alen as i32;

    if resolved.host_str() == base.host_str() {
        score += 15;
    }

    if img_only_low_text {
        score -= 35;
    }

    // Very short anchors on external hosts often mark icons or boilerplate.
    if anchor_text.chars().count() < 2 && resolved.host_str() != base.host_str() {
        score -= 20;
    }

    Some(score)
}

/// Extract and rank `<a href>` targets. Filters obvious media URLs; prefers links with readable anchor text
/// and multi-segment paths. No publisher-specific rules.
pub fn extract_ranked_page_links(html: &str, page_url: &str) -> Vec<WebOutboundLink> {
    let Ok(base) = Url::parse(page_url) else {
        return Vec::new();
    };

    let Ok(sel_a) = Selector::parse("a[href]") else {
        tracing::warn!("link_extract: failed to compile a[href] selector");
        return Vec::new();
    };
    let Ok(sel_img) = Selector::parse("img") else {
        tracing::warn!("link_extract: failed to compile img selector");
        return Vec::new();
    };

    let doc = Html::parse_document(html);
    let mut scored: Vec<(Url, i32, String)> = Vec::new();

    for element in doc.select(&sel_a) {
        let Some(href_raw) = element.value().attr("href") else {
            continue;
        };
        let Some(resolved) = resolve_anchor(&base, href_raw) else {
            continue;
        };
        if skip_scheme_or_void(&resolved) {
            continue;
        }

        let text = anchor_text_for_scoring(element);
        let img_only = img_only_anchor(&sel_img, element, &text);

        let Some(s) = score_candidate(&resolved, &text, &base, img_only) else {
            continue;
        };
        if s < 1 {
            continue;
        }

        scored.push((resolved, s, text));
    }

    scored.sort_by(|a, b| b.1.cmp(&a.1));

    let mut seen = HashSet::<String>::new();
    let mut ranked: Vec<WebOutboundLink> = Vec::new();

    for (url, _s, text) in scored {
        let key = url.to_string();
        if !seen.insert(key) {
            continue;
        }
        let anchor_text = if text.is_empty() {
            None
        } else {
            Some(text)
        };
        ranked.push(WebOutboundLink {
            url: url.to_string(),
            anchor_text,
            rank: 0,
        });
        if ranked.len() >= OUTBOUND_LINK_CAP {
            break;
        }
    }

    for (i, link) in ranked.iter_mut().enumerate() {
        link.rank = (i + 1) as u32;
    }

    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_article_path_over_image_href() {
        let html = r#"<!DOCTYPE html>
<html><body>
<a href="https://news.example.org/2024/world/sea-level-report">Report on coastal cities</a>
<a href="https://cdn.example.org/i/story-800x600.jpg"><img alt="x"/></a>
<a href="/static/logo.svg">Home</a>
</body></html>"#;
        let links = extract_ranked_page_links(html, "https://news.example.org/");
        let urls: Vec<&str> = links.iter().map(|l| l.url.as_str()).collect();
        assert!(
            urls.iter().any(|u| u.contains("/2024/world/sea-level-report")),
            "expected article path in ranked links: {:?}",
            urls
        );
        assert!(
            !urls.iter().any(|u| u.contains(".jpg")),
            "image URL should be dropped: {:?}",
            urls
        );
        assert_eq!(links[0].rank, 1);
    }

    #[test]
    fn drops_mailto_and_fragments() {
        let html = r##"<a href="mailto:a@b">m</a><a href="#x">t</a><a href="/p/q">Real</a>"##;
        let base = "https://example.com/";
        let links = extract_ranked_page_links(html, base);
        assert_eq!(links.len(), 1);
        assert!(links[0].url.ends_with("/p/q"));
    }

    #[test]
    fn dimension_filename_filtered_even_without_common_ext() {
        let html =
            r#"<a href="https://x.test/cdn/abc-300x200.webp">x</a><a href="/article/slug">Title here</a>"#;
        let links = extract_ranked_page_links(html, "https://x.test/");
        assert!(links.iter().all(|l| !l.url.contains("300x200")));
        assert!(links.iter().any(|l| l.url.contains("/article/slug")));
    }

    #[test]
    fn resolves_root_relative_path_like_taz_heise() {
        let html = r##"<a href="/Francesca-Albanese-ueber-ihre-Arbeit/!6170795/" class="teaser">x</a>"##;
        let links = extract_ranked_page_links(html, "https://www.taz.de/zeitung/");
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].url,
            "https://www.taz.de/Francesca-Albanese-ueber-ihre-Arbeit/!6170795/"
        );
        let html_heise = r##"<a href="/bestenlisten/testbericht/slug/9kd8dgw?wt_mc=x" title="Long headline for keyboard">y</a>"##;
        let h = extract_ranked_page_links(html_heise, "https://www.heise.de/newsticker/");
        assert!(h.iter().any(|l| l.url.starts_with("https://www.heise.de/bestenlisten/")));
        assert!(h[0].anchor_text.as_deref().is_some_and(|t| t.contains("Long headline")));
    }

    #[test]
    fn uses_title_when_visible_anchor_text_is_tiny() {
        let html = r##"<a href="/politik/a/2026/x" title="US-Soldaten in Deutschland: Teilabzug"><img alt=""/></a>"##;
        let links = extract_ranked_page_links(html, "https://www.zeit.de/");
        assert_eq!(links.len(), 1);
        assert!(links[0].url.contains("zeit.de/politik"));
        assert!(
            links[0]
                .anchor_text
                .as_deref()
                .is_some_and(|t| t.contains("US-Soldaten")),
            "{:?}",
            links[0].anchor_text
        );
    }
}
