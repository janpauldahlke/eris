//! Outbound link shaping for fetch receipts.

use crate::tools::web::artifact::WebOutboundLink;
use crate::tools::web::ledger::normalize_host;
use url::Url;

/// Receipt / explore hints on generic `web:fetch` (keeps context small).
pub const INTERNAL_LINK_CAP: usize = 3;
/// Homepage fetches for `news:today` — enough headlines without bloating receipts.
pub const HEADLINE_LINK_CAP: usize = 12;

/// Resolve relative `href` values from browser39 against the fetched page URL.
pub fn absolutize_outbound_links(
    links: Vec<WebOutboundLink>,
    page_url: &str,
) -> Vec<WebOutboundLink> {
    let Ok(base) = Url::parse(page_url) else {
        return links;
    };
    links
        .into_iter()
        .filter_map(|mut link| {
            let resolved = if Url::parse(&link.url).is_ok() {
                link.url.clone()
            } else {
                base.join(link.url.trim())
                    .ok()
                    .map(|u| u.to_string())?
            };
            if !resolved.starts_with("http://") && !resolved.starts_with("https://") {
                return None;
            }
            link.url = resolved;
            Some(link)
        })
        .collect()
}

/// Keep browser39 order; optionally prefer same-host links first (MVP host filter only).
pub fn filter_same_host_links(
    links: Vec<WebOutboundLink>,
    page_url: &str,
) -> Vec<WebOutboundLink> {
    let page_host = Url::parse(page_url)
        .ok()
        .and_then(|u| u.host_str().map(normalize_host));
    let Some(page_host) = page_host else {
        return links;
    };
    links
        .into_iter()
        .filter(|l| {
            Url::parse(&l.url)
                .ok()
                .and_then(|u| u.host_str().map(normalize_host))
                .is_some_and(|h| h == page_host)
        })
        .collect()
}

/// Rank by token overlap with `mission_note`, then take up to `cap` links.
pub fn rank_internal_links(
    links: Vec<WebOutboundLink>,
    mission_note: Option<&str>,
) -> Vec<WebOutboundLink> {
    rank_internal_links_with_cap(links, mission_note, INTERNAL_LINK_CAP)
}

pub fn rank_internal_links_with_cap(
    mut links: Vec<WebOutboundLink>,
    mission_note: Option<&str>,
    cap: usize,
) -> Vec<WebOutboundLink> {
    if let Some(note) = mission_note.filter(|n| !n.trim().is_empty()) {
        let tokens = tokenize(note);
        if !tokens.is_empty() {
            links.sort_by(|a, b| {
                link_score(b, &tokens)
                    .cmp(&link_score(a, &tokens))
                    .then_with(|| a.rank.cmp(&b.rank))
            });
        }
    }
    links.into_iter().take(cap.max(1)).collect()
}

/// Higher link cap for `news:today` homepage missions.
pub fn is_news_today_homepage_mission(mission_note: Option<&str>) -> bool {
    mission_note
        .map(str::trim)
        .is_some_and(|n| n.starts_with("news:today"))
}

/// Drop site-root, skip-links, and section-hub rows before storing headline links.
pub fn filter_headline_candidates(
    links: Vec<WebOutboundLink>,
    page_url: &str,
) -> Vec<WebOutboundLink> {
    let Ok(homepage) = Url::parse(page_url) else {
        return links;
    };
    let strict: Vec<_> = links
        .iter()
        .filter(|l| !is_low_value_headline_link(l, &homepage))
        .cloned()
        .collect();
    if strict.iter().any(is_article_like_url) {
        return strict;
    }
    if !strict.is_empty() {
        return strict;
    }
    // Homepage only surfaced section nav (see BBC /news links.json) — keep non-root links.
    links
        .into_iter()
        .filter(|l| !is_site_root_or_skip_link(l, &homepage))
        .collect()
}

fn is_site_root_or_skip_link(link: &WebOutboundLink, homepage: &Url) -> bool {
    let Ok(u) = Url::parse(&link.url) else {
        return true;
    };
    let path = u.path();
    if path == "/" || path.is_empty() {
        return true;
    }
    if path == homepage.path() {
        return true;
    }
    if let Some(anchor) = link.anchor_text.as_deref() {
        let a = anchor.to_lowercase();
        if a.contains("skip to")
            || a.contains("zum inhalt")
            || a.contains("springen")
            || a.contains("british broadcasting corporation")
        {
            return true;
        }
    }
    false
}

/// Prefer article-like URLs, then cap (used for `news:today` homepage storage).
pub fn rank_headline_links(
    links: Vec<WebOutboundLink>,
    page_url: &str,
    mission_note: Option<&str>,
    cap: usize,
) -> Vec<WebOutboundLink> {
    let Ok(homepage) = Url::parse(page_url) else {
        return rank_internal_links_with_cap(links, mission_note, cap);
    };
    let mut links = filter_headline_candidates(links, page_url);
    if let Some(note) = mission_note.filter(|n| !n.trim().is_empty()) {
        let tokens = tokenize(note);
        if !tokens.is_empty() {
            links.sort_by(|a, b| {
                link_score(b, &tokens)
                    .cmp(&link_score(a, &tokens))
                    .then_with(|| a.rank.cmp(&b.rank))
            });
        }
    }
    links.sort_by(|a, b| {
        article_link_score(b, &homepage)
            .cmp(&article_link_score(a, &homepage))
            .then_with(|| a.rank.cmp(&b.rank))
    });
    links.into_iter().take(cap.max(1)).collect()
}

/// Subset of headline links suitable for `news:today` deep fetch (no site root / section hubs).
pub fn select_deep_fetch_links(
    links: &[WebOutboundLink],
    homepage_url: &str,
    max: usize,
) -> Vec<WebOutboundLink> {
    let Ok(homepage) = Url::parse(homepage_url) else {
        return links.iter().take(max).cloned().collect();
    };
    let mut candidates: Vec<_> = links
        .iter()
        .filter(|l| !is_low_value_headline_link(l, &homepage) && is_article_like_url(l))
        .cloned()
        .collect();
    candidates.sort_by(|a, b| {
        article_link_score(b, &homepage)
            .cmp(&article_link_score(a, &homepage))
            .then_with(|| a.rank.cmp(&b.rank))
    });
    candidates.into_iter().take(max.max(1)).collect()
}

fn is_low_value_headline_link(link: &WebOutboundLink, homepage: &Url) -> bool {
    if is_site_root_or_skip_link(link, homepage) {
        return true;
    }
    let Ok(u) = Url::parse(&link.url) else {
        return true;
    };
    let path = u.path();
    let segments: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if segments.len() == 1 {
        let seg = segments[0].to_lowercase();
        if matches!(
            seg.as_str(),
            "sport" | "business"
                | "technology"
                | "health"
                | "culture"
                | "arts"
                | "travel"
                | "audio"
                | "news"
                | "schlagzeilen"
                | "plus"
                | "debatten"
                | "games"
                | "magazin"
                | "politik"
                | "themen"
                | "fuermich"
                | "spiegel"
        ) {
            return true;
        }
    }
    if segments.len() == 2 {
        let seg0 = segments[0].to_lowercase();
        if seg0 == "news" {
            let seg1 = segments[1].to_lowercase();
            if matches!(
                seg1.as_str(),
                "sport" | "business" | "technology" | "health" | "culture" | "arts" | "travel"
            ) {
                return true;
            }
        }
    }
    if u.host_str().is_some_and(|h| h.contains("taz.de")) {
        let lower = link.url.to_lowercase();
        if lower.contains("taz-zahl-ich") || lower.contains("/themen/") {
            return true;
        }
        if path.contains("!p") && !path.contains("!t") {
            return true;
        }
    }
    if matches!(path, "/schlagzeilen/" | "/spiegel/" | "/fuermich/" | "/debatten/" | "/games/") {
        return true;
    }
    false
}

fn is_article_like_url(link: &WebOutboundLink) -> bool {
    let Ok(u) = Url::parse(&link.url) else {
        return false;
    };
    let path = u.path();
    if path.contains("/articles/") {
        return true;
    }
    if path.contains("!t") && path.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    if path.contains("/artikel/") && !path.ends_with("/artikel/") {
        return true;
    }
    let segments: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if segments.len() >= 3 {
        let joined = path.to_lowercase();
        if joined.contains("/thema/") || joined.contains("/politik/") {
            return true;
        }
        return true;
    }
    false
}

fn article_link_score(link: &WebOutboundLink, _homepage: &Url) -> i32 {
    let Ok(u) = Url::parse(&link.url) else {
        return 0;
    };
    let path = u.path();
    let segments: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let mut score = segments.len() as i32;
    if path.contains("/articles/") {
        score += 12;
    }
    if path.contains("!t") {
        score += 10;
    }
    if path.contains("/artikel/") {
        score += 8;
    }
    if path.contains("/thema/") && segments.len() >= 2 {
        score += 6;
    }
    score
}

fn link_score(link: &WebOutboundLink, tokens: &[String]) -> usize {
    let hay = format!(
        "{} {}",
        link.url.to_lowercase(),
        link.anchor_text.as_deref().unwrap_or("").to_lowercase()
    );
    tokens.iter().filter(|t| hay.contains(t.as_str())).count()
}

fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|s| s.to_lowercase())
        .filter(|s| s.len() >= 3)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolutize_relative_href_against_page() {
        let links = vec![WebOutboundLink {
            url: "/news/world".into(),
            anchor_text: Some("World".into()),
            rank: 1,
        }];
        let out = absolutize_outbound_links(links, "https://www.bbc.com/");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://www.bbc.com/news/world");
    }

    #[test]
    fn caps_to_three_links() {
        let links: Vec<_> = (0..5)
            .map(|i| WebOutboundLink {
                url: format!("https://example.com/{i}"),
                anchor_text: None,
                rank: i,
            })
            .collect();
        assert_eq!(rank_internal_links(links, None).len(), 3);
    }

    #[test]
    fn filters_bbc_root_from_headline_candidates() {
        let homepage = Url::parse("https://www.bbc.com/news").expect("url");
        let links = vec![
            WebOutboundLink {
                url: "https://www.bbc.com/".into(),
                anchor_text: Some("British Broadcasting Corporation".into()),
                rank: 1,
            },
            WebOutboundLink {
                url: "https://www.bbc.com/news/articles/c2l2p0wwzzdo".into(),
                anchor_text: Some("Ebola outbreak".into()),
                rank: 50,
            },
        ];
        let filtered: Vec<_> = filter_headline_candidates(links, homepage.as_str())
            .into_iter()
            .map(|l| l.url)
            .collect();
        assert!(!filtered.contains(&"https://www.bbc.com/".to_string()));
        assert!(filtered
            .iter()
            .any(|u| u.contains("/news/articles/")));
    }

    #[test]
    fn select_deep_fetch_skips_bbc_root() {
        let links = vec![
            WebOutboundLink {
                url: "https://www.bbc.com/".into(),
                anchor_text: Some("BBC".into()),
                rank: 1,
            },
            WebOutboundLink {
                url: "https://www.bbc.com/news/articles/abc".into(),
                anchor_text: Some("Story".into()),
                rank: 2,
            },
        ];
        let picked = select_deep_fetch_links(&links, "https://www.bbc.com/news", 1);
        assert_eq!(picked.len(), 1);
        assert!(picked[0].url.contains("/articles/"));
    }

    #[test]
    fn headline_cap_allows_twelve() {
        let links: Vec<_> = (0..20)
            .map(|i| WebOutboundLink {
                url: format!("https://example.com/{i}"),
                anchor_text: None,
                rank: i,
            })
            .collect();
        assert_eq!(
            rank_internal_links_with_cap(links, Some("news:today homepage"), HEADLINE_LINK_CAP).len(),
            HEADLINE_LINK_CAP
        );
    }
}
