//! The user's own lists of hosts for the browser tools: hosts that are
//! refused before a tab is opened or a script navigates to them, and
//! hosts to prefer for literature and similar retrieval.
//!
//! The rules follow `src-tauri/src/browser_url_filters.rs`, which cannot
//! be shared from here because it lives in the Tauri shell, and the lists
//! are kept under the same settings key, so both desktops use one list.

use std::sync::OnceLock;

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;
use wisp_store::Store;

const KEY: &str = "browser_url_filters";
pub const MAX_RULES: usize = 200;
const MAX_REASON_CHARS: usize = 240;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserHostRule {
    pub host: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserFilters {
    #[serde(default)]
    pub block: Vec<BrowserHostRule>,
    #[serde(default)]
    pub prefer: Vec<BrowserHostRule>,
}

impl BrowserFilters {
    /// The rule that refuses `url`, when one does. A host matches itself
    /// and every subdomain.
    pub fn blocked(&self, url: &str) -> Option<&BrowserHostRule> {
        let host = url_host(url)?;
        self.block
            .iter()
            .find(|rule| host_matches(&host, &rule.host))
    }

    pub fn is_preferred(&self, url: &str) -> bool {
        url_host(url).is_some_and(|host| {
            self.prefer
                .iter()
                .any(|rule| host_matches(&host, &rule.host))
        })
    }

    /// The first address a script would navigate to that is refused.
    pub fn blocked_navigation(&self, script: &str) -> Option<(String, &BrowserHostRule)> {
        navigation_urls(script)
            .into_iter()
            .find_map(|url| self.blocked(&url).map(|rule| (url.clone(), rule)))
    }
}

pub fn block_message(url: &str, rule: &BrowserHostRule) -> String {
    if rule.reason.is_empty() {
        format!("blocked by user URL filter: {url} (host {})", rule.host)
    } else {
        format!(
            "blocked by user URL filter: {url} (host {}; {})",
            rule.host, rule.reason
        )
    }
}

/// The host a rule names: a bare domain or an address, lower case,
/// without a leading `*.`.
pub fn parse_rule_host(input: &str) -> Result<String, &'static str> {
    let trimmed = input.trim().trim_start_matches("*.").trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("host is required");
    }
    let parsed = if trimmed.contains("://") {
        Url::parse(trimmed)
    } else {
        Url::parse(&format!("https://{trimmed}"))
    }
    .map_err(|_| "host is not a valid domain or URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("only http(s) hosts can be filtered");
    }
    let host = parsed
        .host_str()
        .ok_or("host is required")?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err("host is required");
    }
    Ok(host)
}

fn url_host(url: &str) -> Option<String> {
    let parsed = Url::parse(url.trim()).ok()?;
    matches!(parsed.scheme(), "http" | "https").then_some(())?;
    Some(
        parsed
            .host_str()?
            .trim_end_matches('.')
            .to_ascii_lowercase(),
    )
}

fn host_matches(url_host: &str, rule_host: &str) -> bool {
    if rule_host.is_empty() || url_host.is_empty() {
        return false;
    }
    url_host == rule_host
        || url_host
            .len()
            .checked_sub(rule_host.len() + 1)
            .is_some_and(|split| {
                url_host.as_bytes().get(split) == Some(&b'.') && url_host[split + 1..] == *rule_host
            })
}

/// The addresses a script names as where it goes: a tab or DevTools
/// command's address, an assignment to `location`, or `window.open`.
pub fn navigation_urls(script: &str) -> Vec<String> {
    let trimmed = script.trim();
    let mut urls = vec![];
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let cmd = value.get("cmd").and_then(Value::as_str).unwrap_or("");
        if cmd.eq_ignore_ascii_case("tabs") {
            urls.extend(value.get("url").and_then(Value::as_str).map(String::from));
        }
        if cmd.eq_ignore_ascii_case("cdp")
            && matches!(
                value.get("method").and_then(Value::as_str),
                Some("Page.navigate" | "Target.createTarget")
            )
        {
            urls.extend(
                value
                    .pointer("/params/url")
                    .and_then(Value::as_str)
                    .map(String::from),
            );
        }
    }
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    let regex = REGEX.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)\b(?:(?:window|document)\.)?location(?:\.href)?\s*=\s*['"](https?://[^'"]+)['"]|\b(?:(?:window|document)\.)?location\.(?:assign|replace)\s*\(\s*['"](https?://[^'"]+)['"]|\bwindow\.open\s*\(\s*['"](https?://[^'"]+)['"]"#,
        )
        .expect("navigation regex")
    });
    for captures in regex.captures_iter(trimmed) {
        urls.extend(
            (1..captures.len())
                .filter_map(|index| captures.get(index))
                .map(|found| found.as_str().to_owned())
                .filter(|url| !url.is_empty()),
        );
    }
    urls
}

/// Rules as they are kept: hosts normalized, a host listed once (its
/// latest reason wins), reasons trimmed and bounded. A rule whose host
/// cannot be read is dropped.
fn normalize(rules: Vec<BrowserHostRule>) -> Vec<BrowserHostRule> {
    let mut out: Vec<BrowserHostRule> = vec![];
    for rule in rules {
        let Ok(host) = parse_rule_host(&rule.host) else {
            continue;
        };
        let reason: String = rule.reason.trim().chars().take(MAX_REASON_CHARS).collect();
        match out.iter_mut().find(|kept| kept.host == host) {
            Some(kept) => kept.reason = reason,
            None => out.push(BrowserHostRule { host, reason }),
        }
    }
    out
}

pub(crate) async fn load(store: &Store) -> BrowserFilters {
    store
        .get_setting(KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<BrowserFilters>(&raw).ok())
        .map(|filters| BrowserFilters {
            block: normalize(filters.block),
            prefer: normalize(filters.prefer),
        })
        .unwrap_or_default()
}

/// Keeps the lists. A host that cannot be read refuses the whole save,
/// so a mistyped rule is not silently dropped.
pub(crate) async fn save(store: &Store, filters: BrowserFilters) -> Result<BrowserFilters> {
    for rule in filters.block.iter().chain(&filters.prefer) {
        if let Err(problem) = parse_rule_host(&rule.host) {
            anyhow::bail!("{}: {problem}", rule.host.trim());
        }
    }
    let filters = BrowserFilters {
        block: normalize(filters.block),
        prefer: normalize(filters.prefer),
    };
    ensure!(
        filters.block.len() <= MAX_RULES && filters.prefer.len() <= MAX_RULES,
        "at most {MAX_RULES} hosts per list"
    );
    store
        .set_setting(KEY, &serde_json::to_string(&filters)?)
        .await?;
    Ok(filters)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(host: &str, reason: &str) -> BrowserHostRule {
        BrowserHostRule {
            host: host.into(),
            reason: reason.into(),
        }
    }

    #[test]
    fn a_host_is_read_from_a_domain_or_an_address() {
        assert_eq!(parse_rule_host(" Example.COM/ "), Ok("example.com".into()));
        assert_eq!(parse_rule_host("*.example.com"), Ok("example.com".into()));
        assert_eq!(
            parse_rule_host("https://Sub.Example.com/path?q=1"),
            Ok("sub.example.com".into())
        );
        for wrong in ["", "  ", "ftp://example.com", "not a host"] {
            assert!(parse_rule_host(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn a_blocked_host_covers_its_subdomains_and_nothing_that_only_ends_alike() {
        let filters = BrowserFilters {
            block: vec![rule("example.com", "taken over")],
            prefer: vec![rule("pubmed.ncbi.nlm.nih.gov", "")],
        };
        assert!(filters.blocked("https://example.com/a").is_some());
        assert!(filters.blocked("http://deep.sub.example.com").is_some());
        assert!(filters.blocked("https://notexample.com").is_none());
        assert!(filters.blocked("https://example.com.evil.org").is_none());
        assert!(filters.blocked("file:///etc/passwd").is_none());
        assert!(filters.is_preferred("https://pubmed.ncbi.nlm.nih.gov/123"));
        assert!(!filters.is_preferred("https://example.org"));
        let found = filters.blocked("https://example.com/a").unwrap();
        assert_eq!(
            block_message("https://example.com/a", found),
            "blocked by user URL filter: https://example.com/a (host example.com; taken over)"
        );
    }

    #[test]
    fn a_script_that_navigates_to_a_blocked_host_is_caught() {
        let filters = BrowserFilters {
            block: vec![rule("example.com", "")],
            prefer: vec![],
        };
        for script in [
            r#"location.href = "https://example.com/x""#,
            r#"window.location = 'https://www.example.com'"#,
            r#"document.location.assign("https://example.com")"#,
            r#"window.open("https://example.com/popup")"#,
            r#"{"cmd":"tabs","method":"create","url":"https://example.com"}"#,
            r#"{"cmd":"cdp","method":"Page.navigate","params":{"url":"https://example.com"}}"#,
        ] {
            assert!(filters.blocked_navigation(script).is_some(), "{script}");
        }
        for script in [
            "document.title",
            r#"location.href = "https://example.org""#,
            r#"fetch("https://example.org/data.json")"#,
        ] {
            assert!(filters.blocked_navigation(script).is_none(), "{script}");
        }
    }

    #[tokio::test]
    async fn the_lists_are_kept_normalized_and_a_bad_host_refuses_the_save() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        assert_eq!(load(&store).await, BrowserFilters::default());
        let kept = save(
            &store,
            BrowserFilters {
                block: vec![
                    rule(" Example.com ", " old "),
                    rule("https://example.com/x", " domain taken over "),
                ],
                prefer: vec![rule("*.nih.gov", "")],
            },
        )
        .await
        .unwrap();
        assert_eq!(kept.block, [rule("example.com", "domain taken over")]);
        assert_eq!(kept.prefer, [rule("nih.gov", "")]);
        assert_eq!(load(&store).await, kept);

        let bad = BrowserFilters {
            block: vec![rule("ftp://example.com", "")],
            prefer: vec![],
        };
        assert!(save(&store, bad).await.is_err());
        let many = BrowserFilters {
            block: (0..=MAX_RULES)
                .map(|n| rule(&format!("h{n}.example"), ""))
                .collect(),
            prefer: vec![],
        };
        assert!(save(&store, many).await.is_err());
        assert_eq!(load(&store).await, kept);
    }
}
