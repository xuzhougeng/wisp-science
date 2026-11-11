//! Headless clients use the same exact-ID catalog and snapshot as the desktop.
use anyhow::{ensure, Context, Result};
use std::sync::OnceLock;
use wisp_llm::{ProviderConfig, ProviderKind};

#[allow(dead_code)]
#[path = "../../../src-tauri/src/model_catalog_shared.rs"]
mod catalog;

fn entry(config: &ProviderConfig) -> Option<&'static catalog::CatalogEntry> {
    static CATALOG: OnceLock<catalog::Catalog> = OnceLock::new();
    let catalog = CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../src-tauri/model_catalog.snapshot.json"
        ))
        .expect("checked-in model catalog must parse")
    });
    let provider = match config.kind {
        ProviderKind::Anthropic => "anthropic",
        _ => "openai",
    };
    catalog::lookup(catalog, provider, &config.base_url, &config.model)
}

pub fn resolve(
    config: &ProviderConfig,
    context: Option<usize>,
    output: Option<u64>,
) -> Result<(usize, u64)> {
    let (context, output) = match entry(config) {
        Some(entry) => {
            let cap = usize::try_from(entry.i.unwrap_or(entry.c).min(entry.c))?;
            (
                context.unwrap_or(cap).min(cap),
                output.unwrap_or(entry.o).min(entry.o),
            )
        }
        None => (
            context.context("Model is not in the exact-ID catalog; set WISP_MAX_CONTEXT")?,
            output.context("Model is not in the exact-ID catalog; set WISP_MAX_TOKENS")?,
        ),
    };
    ensure!(
        context > 0 && output > 0,
        "Model context and output limits must be positive"
    );
    Ok((context, output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_gateway_model_uses_catalog_and_clamps_overrides() {
        let cfg =
            ProviderConfig::openai("https://api.kimi.com/coding/v1", "fake", "vendor/k3-256k");
        assert_eq!(resolve(&cfg, None, None).unwrap(), (262_144, 131_072));
        assert_eq!(
            resolve(&cfg, Some(1_000_000), Some(1_000_000)).unwrap(),
            (262_144, 131_072)
        );
        assert_eq!(
            resolve(&cfg, Some(10_000), Some(1000)).unwrap(),
            (10_000, 1000)
        );
    }

    #[test]
    fn unknown_sibling_needs_explicit_limits() {
        let cfg = ProviderConfig::openai("https://api.moonshot.ai/v1", "fake", "kimi-k3-unknown");
        assert!(resolve(&cfg, None, Some(100)).is_err());
        assert!(resolve(&cfg, Some(1000), None).is_err());
        assert_eq!(resolve(&cfg, Some(1000), Some(100)).unwrap(), (1000, 100));
        assert!(resolve(&cfg, Some(0), Some(100)).is_err());
    }
}
