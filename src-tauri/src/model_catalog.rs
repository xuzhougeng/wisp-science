//! Compiled-in model catalog: models.dev limits distilled by `build.rs`
//! (offline snapshot fallback). Exact-match lookup backs the context/output
//! clamps in `models.rs` and the settings form auto-fill.

use crate::model_catalog_shared::{Catalog, CatalogEntry};
use serde::Serialize;
use std::sync::OnceLock;

static CATALOG: OnceLock<Catalog> = OnceLock::new();

/// Baked catalog, parsed once on first use.
pub fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("OUT_DIR"),
            "/model_catalog.json"
        )))
        .expect("baked model catalog must parse")
    })
}

pub fn lookup(provider: &str, api_url: &str, model: &str) -> Option<&'static CatalogEntry> {
    crate::model_catalog_shared::lookup(catalog(), provider, api_url, model)
}

/// The model's own output ceiling, when the catalog knows this exact model id.
/// Callers use it to cap a one-shot job's budget at what the model will accept.
pub fn output_tokens(provider: &str, api_url: &str, model: &str) -> Option<u64> {
    lookup(provider, api_url, model)
        .map(|entry| entry.o)
        .filter(|tokens| *tokens >= 16)
}

/// Settings-form projection of one catalog entry.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntryDto {
    pub context_window: u64,
    pub max_tokens: u64,
    pub input_limit: Option<u64>,
    pub supports_vision: bool,
    pub efforts: Vec<String>,
}

#[tauri::command]
pub fn model_catalog_lookup(
    provider: String,
    api_url: String,
    model: String,
) -> Option<CatalogEntryDto> {
    lookup(&provider, &api_url, &model).map(|entry| CatalogEntryDto {
        context_window: entry.c,
        max_tokens: entry.o,
        input_limit: entry.i,
        supports_vision: entry.v,
        efforts: entry.e.clone(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn baked_catalog_parses_and_covers_key_models() {
        let catalog = super::catalog();
        assert!(
            catalog.models.len() > 100,
            "catalog should cover many vendors"
        );
        assert!(
            catalog.api.len() > 100,
            "snapshot records models.dev api base URLs"
        );
        // Regression anchor: this entry is what stops the kimi-k3 prefix
        // swallow. If a catalog refresh changes it, update this expectation
        // deliberately alongside scripts/refresh_model_catalog.sh.
        let entry = super::lookup("openai", "https://api.kimi.com/coding/v1", "k3-256k")
            .expect("k3-256k must resolve under kimi-for-coding");
        assert_eq!((entry.c, entry.o), (262_144, 131_072));
        // No hand-written row knows api.z.ai: the namespace comes from the
        // baked `api` map alone (`zai` -> https://api.z.ai/api/paas/v4).
        let entry = super::lookup("openai", "https://api.z.ai/api/paas/v4", "glm-5.1")
            .expect("glm-5.1 must resolve under zai via the derived host map");
        assert_eq!((entry.c, entry.o), (200_000, 131_072));
    }
}
