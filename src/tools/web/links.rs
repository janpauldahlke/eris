//! Outbound link shaping for fetch receipts.

use crate::tools::web::artifact::WebOutboundLink;
use crate::tools::web::ledger::normalize_host;
use url::Url;

const INTERNAL_LINK_CAP: usize = 3;

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

/// Rank by token overlap with `mission_note`, then take top `INTERNAL_LINK_CAP`.
pub fn rank_internal_links(
    mut links: Vec<WebOutboundLink>,
    mission_note: Option<&str>,
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
    links.into_iter().take(INTERNAL_LINK_CAP).collect()
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
}
