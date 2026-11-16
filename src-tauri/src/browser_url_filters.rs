//! User-defined URL filters for real-browser tools.
//!
//! Block rules are enforced in Rust before `web_open_tab` / navigational
//! `web_execute_js` run. Prefer rules are advisory: they are returned by
//! `browser_setup` and annotated on successful `web_open_tab` results.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;
use tauri::State;
use url::Url;
use wisp_store::Store;

pub const SETTING_KEY: &str = "browser_url_filters";
pub use wisp_runs::browser::{
    auto_close_tabs_enabled, auto_launch_enabled, AUTO_CLOSE_TABS_KEY, AUTO_LAUNCH_KEY,
};
const MAX_RULES: usize = 200;
const MAX_REASON_CHARS: usize = 240;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserUrlFilterRule {
    pub host: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserUrlFilters {
    #[serde(default)]
    pub block: Vec<BrowserUrlFilterRule>,
    #[serde(default)]
    pub prefer: Vec<BrowserUrlFilterRule>,
}

impl BrowserUrlFilters {
    pub fn blocked(&self, url: &str) -> Option<&BrowserUrlFilterRule> {
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

    pub fn blocked_navigation(&self, script: &str) -> Option<(String, &BrowserUrlFilterRule)> {
        for url in navigation_urls(script) {
            if let Some(rule) = self.blocked(&url) {
                return Some((url, rule));
            }
        }
        None
    }
}

pub fn block_message(url: &str, rule: &BrowserUrlFilterRule) -> String {
    if rule.reason.is_empty() {
        format!("blocked by user URL filter: {url} (host {})", rule.host)
    } else {
        format!(
            "blocked by user URL filter: {url} (host {}; {})",
            rule.host, rule.reason
        )
    }
}

pub async fn load(store: &Store) -> BrowserUrlFilters {
    store
        .get_setting(SETTING_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .map(normalize_filters)
        .unwrap_or_default()
}

pub async fn save(store: &Store, filters: BrowserUrlFilters) -> Result<BrowserUrlFilters, String> {
    let filters = normalize_filters(filters);
    if filters.block.len() > MAX_RULES || filters.prefer.len() > MAX_RULES {
        return Err(format!("at most {MAX_RULES} hosts per list"));
    }
    let json = serde_json::to_string(&filters).map_err(|error| error.to_string())?;
    store
        .set_setting(SETTING_KEY, &json)
        .await
        .map_err(|error| error.to_string())?;
    Ok(filters)
}

#[tauri::command]
pub async fn get_browser_url_filters(
    state: State<'_, crate::AppState>,
) -> Result<BrowserUrlFilters, String> {
    Ok(load(&state.store).await)
}

#[tauri::command]
pub async fn set_browser_url_filters(
    state: State<'_, crate::AppState>,
    filters: BrowserUrlFilters,
) -> Result<BrowserUrlFilters, String> {
    save(&state.store, filters).await
}

#[tauri::command]
pub async fn get_browser_auto_launch(state: State<'_, crate::AppState>) -> Result<bool, String> {
    Ok(auto_launch_enabled(&state.store).await)
}

#[tauri::command]
pub async fn set_browser_auto_launch(
    state: State<'_, crate::AppState>,
    enabled: bool,
) -> Result<bool, String> {
    state
        .store
        .set_setting(AUTO_LAUNCH_KEY, if enabled { "true" } else { "false" })
        .await
        .map_err(|error| error.to_string())?;
    Ok(enabled)
}

#[tauri::command]
pub async fn get_browser_auto_close_tabs(
    state: State<'_, crate::AppState>,
) -> Result<bool, String> {
    Ok(auto_close_tabs_enabled(&state.store).await)
}

#[tauri::command]
pub async fn set_browser_auto_close_tabs(
    state: State<'_, crate::AppState>,
    enabled: bool,
) -> Result<bool, String> {
    state
        .store
        .set_setting(AUTO_CLOSE_TABS_KEY, if enabled { "true" } else { "false" })
        .await
        .map_err(|error| error.to_string())?;
    Ok(enabled)
}

fn normalize_filters(filters: BrowserUrlFilters) -> BrowserUrlFilters {
    BrowserUrlFilters {
        block: normalize_rules(filters.block),
        prefer: normalize_rules(filters.prefer),
    }
}

fn normalize_rules(rules: Vec<BrowserUrlFilterRule>) -> Vec<BrowserUrlFilterRule> {
    let mut out = Vec::new();
    for rule in rules {
        let Ok(host) = parse_rule_host(&rule.host) else {
            continue;
        };
        let reason = truncate_reason(&rule.reason);
        if let Some(existing) = out
            .iter_mut()
            .find(|row: &&mut BrowserUrlFilterRule| row.host == host)
        {
            existing.reason = reason;
        } else {
            out.push(BrowserUrlFilterRule { host, reason });
        }
    }
    out
}

fn truncate_reason(reason: &str) -> String {
    let trimmed = reason.trim();
    if trimmed.chars().count() <= MAX_REASON_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_REASON_CHARS).collect()
}

pub fn parse_rule_host(input: &str) -> Result<String, String> {
    let trimmed = input.trim().trim_start_matches("*.").trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("host is required".into());
    }
    let parsed = if trimmed.contains("://") {
        Url::parse(trimmed)
    } else {
        Url::parse(&format!("https://{trimmed}"))
    }
    .map_err(|_| "host is not a valid domain or URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("only http(s) hosts can be filtered".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "host is required".to_string())?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err("host is required".into());
    }
    Ok(host)
}

fn url_host(url: &str) -> Option<String> {
    let parsed = Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
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
    if url_host == rule_host {
        return true;
    }
    url_host
        .len()
        .checked_sub(rule_host.len() + 1)
        .is_some_and(|split| {
            url_host.as_bytes().get(split) == Some(&b'.') && url_host[split + 1..] == *rule_host
        })
}

pub fn navigation_urls(script: &str) -> Vec<String> {
    let trimmed = script.trim();
    let mut urls = Vec::new();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        collect_json_navigation_urls(&value, &mut urls);
    }
    collect_js_navigation_urls(trimmed, &mut urls);
    urls
}

fn collect_json_navigation_urls(value: &Value, urls: &mut Vec<String>) {
    let cmd = value.get("cmd").and_then(Value::as_str).unwrap_or("");
    if cmd.eq_ignore_ascii_case("tabs") {
        if let Some(url) = value.get("url").and_then(Value::as_str) {
            urls.push(url.to_string());
        }
    }
    if cmd.eq_ignore_ascii_case("cdp") {
        let method = value.get("method").and_then(Value::as_str).unwrap_or("");
        if matches!(method, "Page.navigate" | "Target.createTarget") {
            if let Some(url) = value.pointer("/params/url").and_then(Value::as_str) {
                urls.push(url.to_string());
            }
        }
    }
}

fn collect_js_navigation_urls(script: &str, urls: &mut Vec<String>) {
    let regex = js_navigation_regex();
    for caps in regex.captures_iter(script) {
        for index in 1..caps.len() {
            if let Some(url) = caps
                .get(index)
                .map(|m| m.as_str())
                .filter(|url| !url.is_empty())
            {
                urls.push(url.to_string());
            }
        }
    }
}

fn js_navigation_regex() -> &'static regex::Regex {
    static REGEX: OnceLock<regex::Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)\b(?:(?:window|document)\.)?location(?:\.href)?\s*=\s*['"](https?://[^'"]+)['"]|\b(?:(?:window|document)\.)?location\.(?:assign|replace)\s*\(\s*['"](https?://[^'"]+)['"]|\bwindow\.open\s*\(\s*['"](https?://[^'"]+)['"]"#,
        )
        .expect("js navigation URL regex")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_runs::browser::{parse_auto_close_tabs, parse_auto_launch};

    #[test]
    fn parses_hosts_from_bare_domains_and_urls() {
        assert_eq!(parse_rule_host("Example.COM").unwrap(), "example.com");
        assert_eq!(
            parse_rule_host("https://www.example.com/path").unwrap(),
            "www.example.com"
        );
        assert_eq!(parse_rule_host("*.example.com").unwrap(), "example.com");
        assert_eq!(
            parse_rule_host("example.com:443/foo").unwrap(),
            "example.com"
        );
        assert!(parse_rule_host("").is_err());
        assert!(parse_rule_host("ftp://example.com").is_err());
    }

    #[test]
    fn block_matches_host_and_subdomains_only() {
        let filters = BrowserUrlFilters {
            block: vec![BrowserUrlFilterRule {
                host: "example.com".into(),
                reason: "hijacked".into(),
            }],
            prefer: vec![],
        };
        assert_eq!(
            filters.blocked("https://example.com/a").unwrap().reason,
            "hijacked"
        );
        assert!(filters.blocked("https://www.example.com").is_some());
        assert!(filters.blocked("http://docs.www.example.com").is_some());
        assert!(filters.blocked("https://evil-example.com").is_none());
        assert!(filters.blocked("https://example.com.evil.test").is_none());
        assert!(filters.blocked("https://pubmed.ncbi.nlm.nih.gov").is_none());
    }

    #[test]
    fn prefer_is_advisory_and_does_not_block() {
        let filters = BrowserUrlFilters {
            block: vec![],
            prefer: vec![BrowserUrlFilterRule {
                host: "pubmed.ncbi.nlm.nih.gov".into(),
                reason: "literature".into(),
            }],
        };
        assert!(filters.is_preferred("https://pubmed.ncbi.nlm.nih.gov/123"));
        assert!(filters.is_preferred("https://www.pubmed.ncbi.nlm.nih.gov/"));
        assert!(!filters.is_preferred("https://scholar.google.com"));
        assert!(filters.blocked("https://scholar.google.com").is_none());
    }

    #[test]
    fn extracts_navigation_urls_from_json_and_js() {
        assert_eq!(
            navigation_urls(r#"{"cmd":"tabs","method":"create","url":"https://blocked.test"}"#),
            vec!["https://blocked.test"]
        );
        assert_eq!(
            navigation_urls(
                r#"{"cmd":"cdp","method":"Page.navigate","params":{"url":"https://blocked.test/x"}}"#
            ),
            vec!["https://blocked.test/x"]
        );
        assert_eq!(
            navigation_urls("location.href='https://blocked.test/js'"),
            vec!["https://blocked.test/js"]
        );
        assert_eq!(
            navigation_urls(r#"window.open("https://blocked.test/open")"#),
            vec!["https://blocked.test/open"]
        );
        assert!(navigation_urls("document.querySelector('a').click()").is_empty());
        assert!(navigation_urls(r#"{"cmd":"tabs","method":"close","tabIds":[1]}"#).is_empty());
    }

    #[test]
    fn block_message_includes_optional_reason() {
        let with_reason = BrowserUrlFilterRule {
            host: "blocked.test".into(),
            reason: "domain hijacked".into(),
        };
        assert!(block_message("https://blocked.test", &with_reason).contains("domain hijacked"));
        let no_reason = BrowserUrlFilterRule {
            host: "blocked.test".into(),
            reason: String::new(),
        };
        assert!(!block_message("https://blocked.test", &no_reason).contains(';'));
    }

    #[tokio::test]
    async fn save_normalizes_and_round_trips() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_browser_filters_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&tmp).await.unwrap();
        let saved = save(
            &store,
            BrowserUrlFilters {
                block: vec![
                    BrowserUrlFilterRule {
                        host: "https://Blocked.TEST/path".into(),
                        reason: "  taken over  ".into(),
                    },
                    BrowserUrlFilterRule {
                        host: "blocked.test".into(),
                        reason: "duplicate".into(),
                    },
                ],
                prefer: vec![BrowserUrlFilterRule {
                    host: "pubmed.ncbi.nlm.nih.gov".into(),
                    reason: String::new(),
                }],
            },
        )
        .await
        .unwrap();
        assert_eq!(saved.block.len(), 1);
        assert_eq!(saved.block[0].host, "blocked.test");
        assert_eq!(saved.block[0].reason, "duplicate");
        assert_eq!(load(&store).await, saved);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn auto_launch_defaults_on_and_treats_false_as_off() {
        assert!(parse_auto_launch(None));
        assert!(parse_auto_launch(Some("")));
        assert!(parse_auto_launch(Some("true")));
        assert!(!parse_auto_launch(Some("false")));
        assert!(!parse_auto_launch(Some("0")));
        assert!(!parse_auto_launch(Some("off")));
    }

    #[test]
    fn auto_close_tabs_defaults_off_and_treats_true_as_on() {
        assert!(!parse_auto_close_tabs(None));
        assert!(!parse_auto_close_tabs(Some("")));
        assert!(!parse_auto_close_tabs(Some("false")));
        assert!(parse_auto_close_tabs(Some("true")));
        assert!(parse_auto_close_tabs(Some("1")));
        assert!(parse_auto_close_tabs(Some("on")));
    }

    #[tokio::test]
    async fn auto_launch_setting_round_trips() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_browser_auto_launch_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&tmp).await.unwrap();
        assert!(auto_launch_enabled(&store).await);
        store.set_setting(AUTO_LAUNCH_KEY, "false").await.unwrap();
        assert!(!auto_launch_enabled(&store).await);
        store.set_setting(AUTO_LAUNCH_KEY, "true").await.unwrap();
        assert!(auto_launch_enabled(&store).await);
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn auto_close_tabs_setting_round_trips() {
        let tmp = std::env::temp_dir().join(format!(
            "wisp_browser_auto_close_tabs_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&tmp).await.unwrap();
        assert!(!auto_close_tabs_enabled(&store).await);
        store
            .set_setting(AUTO_CLOSE_TABS_KEY, "true")
            .await
            .unwrap();
        assert!(auto_close_tabs_enabled(&store).await);
        store
            .set_setting(AUTO_CLOSE_TABS_KEY, "false")
            .await
            .unwrap();
        assert!(!auto_close_tabs_enabled(&store).await);
        let _ = std::fs::remove_file(&tmp);
    }
}
