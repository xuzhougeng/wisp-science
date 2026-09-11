//! Shared model-catalog logic: types, distillation from the models.dev
//! `api.json`, and exact-match lookup. No IO — this file is compiled both by
//! `build.rs` (via `#[path]`) and by the runtime crate (as a module), so it
//! must stay free of async, filesystem, and network dependencies.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// vendor namespace -> (model id -> entry). BTreeMap keeps the distilled
/// snapshot deterministic for diff-friendly refreshes.
pub type Catalog = BTreeMap<String, BTreeMap<String, CatalogEntry>>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// `limit.context` — total input + output window.
    pub c: u64,
    /// `limit.output` — max output tokens per request.
    pub o: u64,
    /// `limit.input` — split per-request input cap when below the window.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub i: Option<u64>,
    /// Vision: `modalities.input` contains `image`.
    #[serde(skip_serializing_if = "is_false", default)]
    pub v: bool,
    /// Reasoning effort values from `reasoning_options`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub e: Vec<String>,
    /// `[input, output]` USD per 1M tokens.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub p: Option<[f64; 2]>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Distill a full models.dev `api.json` into the compact catalog baked into
/// the binary. Models without a usable `limit.context`/`limit.output` are
/// skipped — a missing output cap must never read as "0 allowed".
pub fn distill(api_json: &str) -> Result<String, String> {
    let root: serde_json::Value =
        serde_json::from_str(api_json).map_err(|e| format!("api.json parse: {e}"))?;
    let providers = root
        .as_object()
        .ok_or_else(|| "api.json root is not an object".to_string())?;
    let mut catalog = Catalog::new();
    for (provider, pdata) in providers {
        let Some(models) = pdata.get("models").and_then(|m| m.as_object()) else {
            continue;
        };
        let mut entries = BTreeMap::new();
        for (id, model) in models {
            let limit = model.get("limit");
            let (Some(c), Some(o)) = (
                limit
                    .and_then(|l| l.get("context"))
                    .and_then(|v| v.as_u64()),
                limit.and_then(|l| l.get("output")).and_then(|v| v.as_u64()),
            ) else {
                continue;
            };
            if c == 0 {
                continue;
            }
            let entry = CatalogEntry {
                c,
                o,
                i: limit
                    .and_then(|l| l.get("input"))
                    .and_then(|v| v.as_u64())
                    .filter(|&i| i > 0),
                v: model
                    .get("modalities")
                    .and_then(|m| m.get("input"))
                    .and_then(|v| v.as_array())
                    .is_some_and(|inputs| inputs.iter().any(|m| m.as_str() == Some("image"))),
                e: model
                    .get("reasoning_options")
                    .and_then(|v| v.as_array())
                    .map(|options| {
                        options
                            .iter()
                            .filter(|option| {
                                option.get("type").and_then(|t| t.as_str()) == Some("effort")
                            })
                            .filter_map(|option| option.get("values").and_then(|v| v.as_array()))
                            .flatten()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                p: match (
                    model
                        .get("cost")
                        .and_then(|c| c.get("input"))
                        .and_then(|v| v.as_f64()),
                    model
                        .get("cost")
                        .and_then(|c| c.get("output"))
                        .and_then(|v| v.as_f64()),
                ) {
                    (Some(input), Some(output)) => Some([input, output]),
                    _ => None,
                },
            };
            entries.insert(id.clone(), entry);
        }
        if !entries.is_empty() {
            catalog.insert(provider.clone(), entries);
        }
    }
    serde_json::to_string(&catalog).map_err(|e| format!("catalog encode: {e}"))
}

/// Lowercase, trimmed model id — the only normalization lookup performs.
pub fn normalize_model_id(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn host_of(api_url: &str) -> String {
    let rest = api_url.trim().split("://").nth(1).unwrap_or(api_url.trim());
    rest.split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Candidate vendor namespaces, most specific first: api_url host mapping,
/// then the profile's protocol-level provider field as a fallback.
pub fn namespace_candidates(provider: &str, api_url: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let host = host_of(api_url);
    let path = api_url
        .trim()
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(api_url.trim())
        .split_once('/')
        .map(|(_, path)| path)
        .unwrap_or("")
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    let by_host = match host.as_str() {
        "opencode.ai" if path == "zen/go" || path.starts_with("zen/go/") => Some("opencode-go"),
        "opencode.ai" if path == "zen" || path.starts_with("zen/") => Some("opencode"),
        "api.anthropic.com" => Some("anthropic"),
        "api.openai.com" => Some("openai"),
        "api.x.ai" => Some("xai"),
        "api.deepseek.com" => Some("deepseek"),
        "api.moonshot.ai" | "api.moonshot.cn" => Some("moonshotai"),
        // Kimi Code managed models live in their own namespace; the same host
        // also serves the open Kimi platform ids.
        "api.kimi.com" => Some("kimi-for-coding"),
        "open.bigmodel.cn" => Some("zhipuai"),
        _ => None,
    };
    if let Some(ns) = by_host {
        out.push(ns);
    }
    let by_provider = match provider.trim() {
        "anthropic" => Some("anthropic"),
        "openai" | "openai_responses" => Some("openai"),
        _ => None,
    };
    if let Some(ns) = by_provider {
        if !out.contains(&ns) {
            out.push(ns);
        }
    }
    out
}

/// Exact-id catalog lookup. No prefix matching: a shorter family id must
/// never absorb a longer sibling (`kimi-k3` vs `k3-256k`, `claude-opus-4` vs
/// `claude-opus-4-1`). Gateway-style `vendor/model` ids also match on their
/// last path segment.
pub fn lookup<'a>(
    catalog: &'a Catalog,
    provider: &str,
    api_url: &str,
    model: &str,
) -> Option<&'a CatalogEntry> {
    let id = normalize_model_id(model);
    if id.is_empty() {
        return None;
    }
    let tail = id.rsplit('/').next().unwrap_or(&id);
    for ns in namespace_candidates(provider, api_url) {
        if let Some(models) = catalog.get(ns) {
            if let Some(entry) = models.get(&id) {
                return Some(entry);
            }
            if tail != id {
                if let Some(entry) = models.get(tail) {
                    return Some(entry);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_catalog_uses_the_go_namespace_and_exact_model_ids() {
        let catalog: Catalog = serde_json::from_str(
            r#"{
            "opencode-go": {"kimi-k3": {"c": 1048576, "o": 131072}},
            "opencode": {"kimi-k3": {"c": 262144, "o": 8192}}
        }"#,
        )
        .unwrap();
        for base in [
            "https://opencode.ai/zen/go",
            "https://OPENCODE.AI/zen/go/v1/",
        ] {
            let entry = lookup(&catalog, "openai", base, "kimi-k3").unwrap();
            assert_eq!(entry.c, 1048576);
            assert!(lookup(&catalog, "openai", base, "kimi-k3-longer").is_none());
        }
        assert_eq!(
            namespace_candidates("anthropic", "https://opencode.ai/zen/go/v1"),
            vec!["opencode-go", "anthropic"]
        );
        assert_eq!(
            namespace_candidates("openai", "https://opencode.ai/zen/v1"),
            vec!["opencode", "openai"]
        );
        assert_eq!(
            namespace_candidates("openai", "https://opencode.ai.evil.test/zen/go/v1"),
            vec!["openai"]
        );
        assert_eq!(
            namespace_candidates("openai", "https://opencode.ai/zen/gopher"),
            vec!["opencode", "openai"]
        );
    }

    fn fixture_catalog() -> Catalog {
        let json = r#"{
            "anthropic": {
                "claude-opus-4-8": { "c": 1000000, "o": 128000, "v": true }
            },
            "kimi-for-coding": {
                "k3-256k": { "c": 262144, "o": 131072, "e": ["low", "high", "max"] }
            },
            "moonshotai": {
                "kimi-k3": { "c": 1048576, "o": 131072, "v": true }
            }
        }"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn xai_host_maps_to_xai_namespace() {
        assert_eq!(
            namespace_candidates("openai", "https://api.x.ai/v1"),
            vec!["xai", "openai"]
        );
    }

    #[test]
    fn lookup_hits_exact_id_via_host_namespace() {
        let catalog = fixture_catalog();
        let entry = lookup(
            &catalog,
            "openai",
            "https://api.kimi.com/coding/v1",
            "k3-256k",
        )
        .expect("k3-256k resolves under kimi-for-coding");
        assert_eq!((entry.c, entry.o), (262_144, 131_072));
    }

    #[test]
    fn lookup_never_swallows_longer_sibling() {
        let catalog = fixture_catalog();
        // "kimi-k3-256k" is not a catalog id; the "kimi-k3" entry must not
        // absorb it via prefix matching.
        assert!(lookup(
            &catalog,
            "openai",
            "https://api.moonshot.ai/v1",
            "kimi-k3-256k"
        )
        .is_none());
        // The real kimi-k3 id still resolves.
        let entry = lookup(&catalog, "openai", "https://api.moonshot.ai/v1", "kimi-k3").unwrap();
        assert_eq!(entry.c, 1_048_576);
    }

    #[test]
    fn lookup_accepts_gateway_vendor_prefixed_ids() {
        let catalog = fixture_catalog();
        let entry = lookup(
            &catalog,
            "openai",
            "https://api.anthropic.com/v1",
            "anthropic/claude-opus-4-8",
        )
        .unwrap();
        assert_eq!(entry.c, 1_000_000);
    }

    #[test]
    fn lookup_falls_back_to_provider_namespace() {
        let catalog = fixture_catalog();
        let entry = lookup(
            &catalog,
            "anthropic",
            "https://proxy.example.com/v1",
            "claude-opus-4-8",
        )
        .unwrap();
        assert_eq!(entry.o, 128_000);
        assert!(lookup(&catalog, "openai", "", "k3-256k").is_none());
    }

    #[test]
    fn lookup_normalizes_case_and_whitespace() {
        let catalog = fixture_catalog();
        assert!(lookup(
            &catalog,
            "openai",
            "https://api.moonshot.ai/v1",
            "  Kimi-K3 "
        )
        .is_some());
        assert!(lookup(&catalog, "openai", "https://api.moonshot.ai/v1", "").is_none());
    }

    #[test]
    fn distill_keeps_limits_and_flags() {
        let api = r#"{
            "vendor": { "models": {
                "m-full": {
                    "id": "m-full",
                    "limit": { "context": 1000000, "output": 128000, "input": 272000 },
                    "modalities": { "input": ["text", "image"], "output": ["text"] },
                    "reasoning_options": [
                        { "type": "effort", "values": ["low", "high"] },
                        { "type": "budget_tokens", "min": 1024 }
                    ],
                    "cost": { "input": 5, "output": 25, "cache_read": 0.5 }
                },
                "m-min": { "id": "m-min", "limit": { "context": 32000, "output": 4096 } },
                "m-no-output": { "id": "m-no-output", "limit": { "context": 32000 } },
                "m-zero": { "id": "m-zero", "limit": { "context": 0, "output": 4096 } }
            } }
        }"#;
        let distilled = distill(api).unwrap();
        let catalog: Catalog = serde_json::from_str(&distilled).unwrap();
        let models = catalog.get("vendor").unwrap();
        assert_eq!(models.len(), 2, "missing/zero limits are skipped");
        let full = &models["m-full"];
        assert_eq!(
            (full.c, full.o, full.i),
            (1_000_000, 128_000, Some(272_000))
        );
        assert!(full.v);
        assert_eq!(full.e, ["low", "high"]);
        assert_eq!(full.p, Some([5.0, 25.0]));
        let min = &models["m-min"];
        assert_eq!(min.i, None);
        assert!(!min.v);
        assert!(min.e.is_empty());
        assert_eq!(min.p, None);
    }
}
