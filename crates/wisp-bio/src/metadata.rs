//! Connector introductions and operator documentation. Tool contracts stay in
//! `catalog()` so the settings page and actual dispatch share one definition.

use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
pub struct DomainLink {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct DomainMetadata {
    pub slug: String,
    pub description: String,
    pub description_zh: String,
    pub links: Vec<DomainLink>,
}

static DOMAINS: LazyLock<Vec<DomainMetadata>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("domains.json"))
        .expect("bundled domain metadata must be valid")
});

pub fn domain_metadata(slug: &str) -> Option<&'static DomainMetadata> {
    DOMAINS.iter().find(|domain| domain.slug == slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_implemented_domain_has_an_introduction_and_operator_links() {
        let implemented: BTreeSet<_> = crate::catalog().into_iter().map(|(slug, _)| slug).collect();
        let described: BTreeSet<_> = DOMAINS.iter().map(|domain| domain.slug.as_str()).collect();
        assert_eq!(implemented, described);
        assert_eq!(described.len(), DOMAINS.len(), "duplicate metadata entries");
        for domain in DOMAINS.iter() {
            assert!(!domain.description.trim().is_empty());
            assert!(!domain.description_zh.trim().is_empty());
            assert!(!domain.links.is_empty());
            for link in &domain.links {
                assert!(!link.label.trim().is_empty());
                let url = reqwest::Url::parse(&link.url).unwrap();
                assert_eq!(url.scheme(), "https");
                assert!(url.host_str().is_some());
                assert!(url.username().is_empty());
                assert!(url.password().is_none());
            }
        }
        assert!(domain_metadata("unknown").is_none());
    }
}
