//! Shared model-catalog logic: types, distillation from the models.dev
//! `api.json`, and exact-match lookup. No IO — this file is compiled both by
//! `build.rs` (via `#[path]`) and by the runtime crate (as a module), so it
//! must stay free of async, filesystem, and network dependencies.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// vendor namespace -> (model id -> entry). BTreeMap keeps the distilled
/// snapshot deterministic for diff-friendly refreshes.
pub type Namespaces = BTreeMap<String, BTreeMap<String, CatalogEntry>>;

/// The baked catalog: model limits per namespace plus each namespace's
/// models.dev `api` base URL, which lets a Base URL host the hand-written
/// table in [`namespace_candidates`] never heard of still find its namespace.
///
/// Serialized as `{"models": {...}, "api": {...}}`. The flat pre-`api` shape
/// (namespaces at the root) still deserializes, so an older committed
/// snapshot keeps building.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "CatalogWire")]
pub struct Catalog {
    pub models: Namespaces,
    /// namespace -> models.dev `api` base URL. Absent for providers that
    /// publish none (`openai`, `anthropic`, `xai`, ...).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub api: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CatalogWire {
    Wide {
        models: Namespaces,
        #[serde(default)]
        api: BTreeMap<String, String>,
    },
    Flat(Namespaces),
}

impl From<CatalogWire> for Catalog {
    fn from(wire: CatalogWire) -> Self {
        match wire {
            CatalogWire::Wide { models, api } => Catalog { models, api },
            CatalogWire::Flat(models) => Catalog {
                models,
                api: BTreeMap::new(),
            },
        }
    }
}

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
/// skipped — a missing output cap must never read as "0 allowed". Each kept
/// namespace also records its `api` base URL when models.dev publishes one.
pub fn distill(api_json: &str) -> Result<String, String> {
    let root: serde_json::Value =
        serde_json::from_str(api_json).map_err(|e| format!("api.json parse: {e}"))?;
    let providers = root
        .as_object()
        .ok_or_else(|| "api.json root is not an object".to_string())?;
    let mut catalog = Catalog::default();
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
        if entries.is_empty() {
            continue;
        }
        catalog.models.insert(provider.clone(), entries);
        if let Some(api) = pdata
            .get("api")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|api| !api.is_empty())
        {
            catalog.api.insert(provider.clone(), api.to_string());
        }
    }
    serde_json::to_string(&catalog).map_err(|e| format!("catalog encode: {e}"))
}

/// Lowercase, trimmed model id — the only normalization lookup performs.
pub fn normalize_model_id(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

/// `(host, path)` of a base URL: host lowercased without port, path without
/// scheme, query, fragment, or trailing slash.
fn host_and_path(url: &str) -> (String, &str) {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    (host, path)
}

/// Local servers all claim the same loopback address (lmstudio, atomic-chat,
/// lynkr, ...), so it identifies nothing.
fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host.starts_with("127.")
        || host == "::1"
        || host.starts_with('[')
}

/// Namespaces whose models.dev `api` host equals `host`, best match first:
/// those whose `api` path is a segment prefix of `path` ordered by prefix
/// length, longest first (so `/api/coding/paas/v4` picks a coding-plan
/// namespace over its `/api/paas/v4` sibling), then the remaining same-host
/// namespaces alphabetically.
fn derived_namespaces<'a>(catalog: &'a Catalog, host: &str, path: &str) -> Vec<&'a str> {
    if host.is_empty() || is_loopback_host(host) {
        return Vec::new();
    }
    let mut matches: Vec<(usize, &str)> = catalog
        .api
        .iter()
        .filter_map(|(ns, api)| {
            let (api_host, api_path) = host_and_path(api);
            if api_host != host {
                return None;
            }
            let is_prefix = api_path.is_empty()
                || path == api_path
                || path
                    .strip_prefix(api_path)
                    .is_some_and(|rest| rest.starts_with('/'));
            Some((if is_prefix { api_path.len() + 1 } else { 0 }, ns.as_str()))
        })
        .collect();
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    matches.into_iter().map(|(_, ns)| ns).collect()
}

/// Candidate vendor namespaces, most specific first: the hand-written host
/// overrides below, then namespaces whose models.dev `api` host matches the
/// Base URL (see [`derived_namespaces`]), then the profile's protocol-level
/// provider field as a fallback.
pub fn namespace_candidates<'a>(
    catalog: &'a Catalog,
    provider: &str,
    api_url: &str,
) -> Vec<&'a str> {
    let mut out: Vec<&'a str> = Vec::new();
    let (host, path) = host_and_path(api_url);
    // Overrides: public URLs that differ from the namespace's `api` field
    // (OpenCode's `/zen` routes), providers that publish no `api` at all, and
    // renamed namespaces an older snapshot may still carry.
    let by_host: &[&'static str] = match host.as_str() {
        "opencode.ai" if path == "zen/go" || path.starts_with("zen/go/") => &["opencode-go"],
        "opencode.ai" if path == "zen" || path.starts_with("zen/") => &["opencode"],
        "api.anthropic.com" => &["anthropic"],
        "api.openai.com" | "chatgpt.com" => &["openai"],
        "api.x.ai" => &["xai"],
        "api.deepseek.com" => &["deepseek"],
        "api.moonshot.ai" | "api.moonshot.cn" => &["moonshotai"],
        // Kimi Code managed models live in their own namespace; the same host
        // also serves the open Kimi platform ids. models.dev renamed that
        // namespace `kimi-for-coding` -> `kimi-code-plan-{cn,global}`, so both
        // are candidates: a freshly fetched catalog carries the new id, a
        // stale offline snapshot still carries the old one.
        "api.kimi.com" => &["kimi-code-plan-cn", "kimi-for-coding"],
        "api.kimi.ai" => &["kimi-code-plan-global", "kimi-for-coding"],
        "open.bigmodel.cn" => &["zhipuai"],
        // Command Code is an OpenAI-compatible gateway with its own models.dev
        // namespace; the `openai` provider fallback knows none of its ids.
        "api.commandcode.ai" => &["commandcode"],
        _ => &[],
    };
    out.extend_from_slice(by_host);
    for ns in derived_namespaces(catalog, &host, path) {
        if !out.contains(&ns) {
            out.push(ns);
        }
    }
    let by_provider = match provider.trim() {
        "anthropic" => Some("anthropic"),
        "openai" | "openai_responses" | "openai_codex" | "openai_chatgpt" => Some("openai"),
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
    for ns in namespace_candidates(catalog, provider, api_url) {
        if let Some(models) = catalog.models.get(ns) {
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
            namespace_candidates(&catalog, "anthropic", "https://opencode.ai/zen/go/v1"),
            vec!["opencode-go", "anthropic"]
        );
        assert_eq!(
            namespace_candidates(&catalog, "openai", "https://opencode.ai/zen/v1"),
            vec!["opencode", "openai"]
        );
        assert_eq!(
            namespace_candidates(
                &catalog,
                "openai",
                "https://opencode.ai.evil.test/zen/go/v1"
            ),
            vec!["openai"]
        );
        assert_eq!(
            namespace_candidates(&catalog, "openai", "https://opencode.ai/zen/gopher"),
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
    fn kimi_code_host_accepts_the_renamed_and_legacy_namespaces() {
        assert_eq!(
            namespace_candidates(
                &Catalog::default(),
                "openai",
                "https://api.kimi.com/coding/v1"
            ),
            vec!["kimi-code-plan-cn", "kimi-for-coding", "openai"]
        );
        assert_eq!(
            namespace_candidates(
                &Catalog::default(),
                "openai",
                "https://api.kimi.ai/coding/v1"
            ),
            vec!["kimi-code-plan-global", "kimi-for-coding", "openai"]
        );
        let renamed: Catalog = serde_json::from_str(
            r#"{"kimi-code-plan-cn": {"k3-256k": {"c": 262144, "o": 131072}}}"#,
        )
        .unwrap();
        let entry = lookup(
            &renamed,
            "openai",
            "https://api.kimi.com/coding/v1",
            "k3-256k",
        )
        .expect("the renamed namespace resolves");
        assert_eq!((entry.c, entry.o), (262_144, 131_072));
    }

    #[test]
    fn commandcode_host_resolves_gateway_ids_in_its_own_namespace() {
        assert_eq!(
            namespace_candidates(
                &Catalog::default(),
                "openai",
                "https://api.commandcode.ai/provider/v1"
            ),
            vec!["commandcode", "openai"]
        );
        let catalog: Catalog = serde_json::from_str(
            r#"{
            "commandcode": {
                "deepseek/deepseek-v4.1-flash": { "c": 1000000, "o": 384000, "e": ["low", "high", "max"] },
                "claude-opus-5-5": { "c": 1000000, "o": 128000, "e": ["low", "medium", "high", "xhigh", "max"] }
            },
            "openai": {
                "gpt-5.6-sol": { "c": 400000, "o": 128000 }
            }
        }"#,
        )
        .unwrap();
        let base = "https://api.commandcode.ai/provider/v1";
        let entry = lookup(&catalog, "openai", base, "deepseek/deepseek-v4.1-flash")
            .expect("vendor-prefixed id resolves under commandcode");
        assert_eq!(entry.e, vec!["low", "high", "max"]);
        let entry = lookup(&catalog, "openai", base, "claude-opus-5-5").unwrap();
        assert_eq!(entry.e, vec!["low", "medium", "high", "xhigh", "max"]);
        // The provider fallback still serves ids the gateway namespace lacks.
        assert_eq!(
            lookup(&catalog, "openai", base, "gpt-5.6-sol").unwrap().c,
            400_000
        );
    }

    #[test]
    fn xai_host_maps_to_xai_namespace() {
        assert_eq!(
            namespace_candidates(&Catalog::default(), "openai", "https://api.x.ai/v1"),
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
            "vendor": { "api": "https://api.vendor.test/v1", "models": {
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
            } },
            "empty": { "api": "https://api.empty.test/v1", "models": {
                "m-no-output": { "id": "m-no-output", "limit": { "context": 32000 } }
            } },
            "no-api": { "models": {
                "m-min": { "id": "m-min", "limit": { "context": 32000, "output": 4096 } }
            } }
        }"#;
        let distilled = distill(api).unwrap();
        assert!(
            distilled.starts_with(r#"{"models":{"#),
            "snapshot uses the wide shape: {distilled}"
        );
        let catalog: Catalog = serde_json::from_str(&distilled).unwrap();
        assert_eq!(
            catalog.api,
            BTreeMap::from([(
                "vendor".to_string(),
                "https://api.vendor.test/v1".to_string()
            )]),
            "api is recorded only for namespaces that kept models"
        );
        assert_eq!(catalog.models.len(), 2);
        let models = catalog.models.get("vendor").unwrap();
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

    /// Namespaces with `api` base URLs the hand-written table knows nothing
    /// about, including the shared-host pairs models.dev publishes.
    fn derived_fixture() -> Catalog {
        serde_json::from_str(
            r#"{
            "models": {
                "newvendor": { "nv-1": { "c": 200000, "o": 32000, "e": ["low", "high"] } },
                "zhipuai": { "glm-5": { "c": 204800, "o": 131072 } },
                "zhipuai-coding-plan": { "glm-5": { "c": 1000000, "o": 131072 } },
                "openai": { "gpt-5": { "c": 400000, "o": 128000 } },
                "lmstudio": { "nv-1": { "c": 4096, "o": 1024 } },
                "lynkr": { "nv-1": { "c": 8192, "o": 2048 } },
                "umans": { "u-1": { "c": 1, "o": 1 } },
                "umans-plan": { "u-1": { "c": 2, "o": 2 } }
            },
            "api": {
                "newvendor": "https://API.NewVendor.test:443/v1/",
                "zhipuai": "https://open.bigmodel.cn/api/paas/v4",
                "zhipuai-coding-plan": "https://open.bigmodel.cn/api/coding/paas/v4",
                "lmstudio": "http://127.0.0.1:1234/v1",
                "lynkr": "http://127.0.0.1:8081/v1",
                "umans": "https://api.umans.test/v1",
                "umans-plan": "https://api.umans.test/v1"
            }
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn wide_and_flat_snapshots_both_parse() {
        let flat: Catalog =
            serde_json::from_str(r#"{"vendor": {"m": {"c": 1000, "o": 100}}}"#).unwrap();
        let wide: Catalog = serde_json::from_str(
            r#"{"models": {"vendor": {"m": {"c": 1000, "o": 100}}},
                "api": {"vendor": "https://api.vendor.test/v1"}}"#,
        )
        .unwrap();
        assert_eq!(flat.models, wide.models);
        assert!(flat.api.is_empty());
        assert_eq!(wide.api["vendor"], "https://api.vendor.test/v1");
        let encoded = serde_json::to_string(&wide).unwrap();
        assert_eq!(serde_json::from_str::<Catalog>(&encoded).unwrap(), wide);
        // An old flat snapshot without `api` re-encodes as the wide shape.
        assert_eq!(
            serde_json::to_string(&flat).unwrap(),
            r#"{"models":{"vendor":{"m":{"c":1000,"o":100}}}}"#
        );
    }

    #[test]
    fn host_only_the_derived_map_knows_resolves_its_namespace() {
        let catalog = derived_fixture();
        assert_eq!(
            namespace_candidates(&catalog, "openai", "https://api.newvendor.test/v1"),
            vec!["newvendor", "openai"]
        );
        // Port and path differences do not matter: the host alone selects it.
        assert_eq!(
            namespace_candidates(
                &catalog,
                "anthropic",
                "https://api.newvendor.test/anthropic"
            ),
            vec!["newvendor", "anthropic"]
        );
        let entry = lookup(&catalog, "openai", "https://api.newvendor.test/v1", "nv-1")
            .expect("nv-1 resolves via the derived host map");
        assert_eq!(entry.c, 200_000);
        assert_eq!(entry.e, ["low", "high"]);
        assert_eq!(
            namespace_candidates(&catalog, "openai", "https://api.unknown.test/v1"),
            vec!["openai"]
        );
        assert_eq!(
            namespace_candidates(&catalog, "openai", "https://sub.api.newvendor.test/v1"),
            vec!["openai"]
        );
    }

    #[test]
    fn shared_host_prefers_the_longest_api_path_prefix() {
        let catalog = derived_fixture();
        // Hand table: open.bigmodel.cn -> zhipuai; the derived map then adds
        // the coding-plan sibling, so both URLs list both namespaces.
        assert_eq!(
            namespace_candidates(&catalog, "openai", "https://open.bigmodel.cn/api/paas/v4"),
            vec!["zhipuai", "zhipuai-coding-plan", "openai"]
        );
        // The derived step alone: the coding-plan path wins for its own URL
        // and anything below it, and loses for the generic one.
        assert_eq!(
            derived_namespaces(&catalog, "open.bigmodel.cn", "api/coding/paas/v4"),
            vec!["zhipuai-coding-plan", "zhipuai"]
        );
        assert_eq!(
            derived_namespaces(&catalog, "open.bigmodel.cn", "api/coding/paas/v4/chat"),
            vec!["zhipuai-coding-plan", "zhipuai"]
        );
        assert_eq!(
            derived_namespaces(&catalog, "open.bigmodel.cn", "api/paas/v4"),
            vec!["zhipuai", "zhipuai-coding-plan"]
        );
        // A prefix must be segment-aligned: `/api/paas/v4x` matches neither,
        // so both fall back to alphabetical order.
        assert_eq!(
            derived_namespaces(&catalog, "open.bigmodel.cn", "api/paas/v4x"),
            vec!["zhipuai", "zhipuai-coding-plan"]
        );
        // Identical `api` URLs stay deterministic (alphabetical).
        assert_eq!(
            derived_namespaces(&catalog, "api.umans.test", "v1"),
            vec!["umans", "umans-plan"]
        );
        let entry = lookup(&catalog, "openai", "https://api.umans.test/v1", "u-1").unwrap();
        assert_eq!(entry.c, 1);
    }

    #[test]
    fn loopback_hosts_are_ignored() {
        let catalog = derived_fixture();
        for base in [
            "http://127.0.0.1:1234/v1",
            "http://localhost:1234/v1",
            "http://app.localhost/v1",
            "http://[::1]:8081/v1",
        ] {
            assert_eq!(
                namespace_candidates(&catalog, "openai", base),
                vec!["openai"],
                "{base}"
            );
            assert!(lookup(&catalog, "openai", base, "nv-1").is_none(), "{base}");
        }
    }

    #[test]
    fn hand_written_table_wins_over_the_derived_map() {
        let mut catalog = derived_fixture();
        // models.dev's `api` for OpenCode is `/inference/openai/v1`, not the
        // public `/zen` routes; the override still picks the Go namespace
        // first, and a conflicting derived entry is only appended.
        catalog.models.insert(
            "opencode-go".to_string(),
            BTreeMap::from([(
                "kimi-k3".to_string(),
                CatalogEntry {
                    c: 1,
                    o: 1,
                    ..Default::default()
                },
            )]),
        );
        catalog.models.insert(
            "opencode".to_string(),
            BTreeMap::from([(
                "kimi-k3".to_string(),
                CatalogEntry {
                    c: 2,
                    o: 2,
                    ..Default::default()
                },
            )]),
        );
        catalog.api.insert(
            "opencode".to_string(),
            "https://opencode.ai/inference/openai/v1".to_string(),
        );
        catalog.api.insert(
            "opencode-go".to_string(),
            "https://opencode.ai/inference/go/openai/v1".to_string(),
        );
        assert_eq!(
            namespace_candidates(&catalog, "openai", "https://opencode.ai/zen/go/v1"),
            vec!["opencode-go", "opencode", "openai"]
        );
        assert_eq!(
            lookup(
                &catalog,
                "openai",
                "https://opencode.ai/zen/go/v1",
                "kimi-k3"
            )
            .unwrap()
            .c,
            1
        );
        assert_eq!(
            namespace_candidates(&catalog, "openai", "https://opencode.ai/zen/v1"),
            vec!["opencode", "opencode-go", "openai"]
        );
        // A derived entry that points a hand-written host elsewhere comes
        // after the override.
        catalog
            .api
            .insert("impostor".to_string(), "https://api.x.ai/v1".to_string());
        assert_eq!(
            namespace_candidates(&catalog, "openai", "https://api.x.ai/v1"),
            vec!["xai", "impostor", "openai"]
        );
    }

    #[test]
    fn host_and_path_strips_scheme_port_credentials_and_query() {
        assert_eq!(
            host_and_path("https://user:pw@API.Example.test:8443/a/b/?x=1#f"),
            ("api.example.test".to_string(), "a/b")
        );
        assert_eq!(
            host_and_path("api.example.test"),
            ("api.example.test".to_string(), "")
        );
        assert_eq!(host_and_path(""), (String::new(), ""));
    }
}
