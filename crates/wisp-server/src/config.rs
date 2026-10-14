use anyhow::{bail, ensure, Context, Result};
use std::{net::SocketAddr, path::PathBuf};
use wisp_llm::{ProviderConfig, ProviderKind};
use wisp_sync::RemoteCode;

// Use the exact lookup and checked-in catalog also used by the desktop. No
// network fetch or Tauri build is required for the server.
#[allow(dead_code)]
#[path = "../../../src-tauri/src/model_catalog_shared.rs"]
mod catalog;

#[derive(Clone)]
pub struct Config {
    pub data: PathBuf,
    pub workspace: PathBuf,
    pub bind: SocketAddr,
    pub public_url: String,
    pub code: RemoteCode,
    pub provider: ProviderConfig,
    pub max_context: usize,
    pub supports_vision: bool,
    pub max_iterations: usize,
    pub skills: Vec<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::parse(|name| std::env::var(name).ok())
    }

    fn parse(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let required = |name| {
            get(name)
                .filter(|v| !v.trim().is_empty())
                .with_context(|| format!("{name} is required"))
        };
        let public_url = required("WISP_SERVER_PUBLIC_URL")?;
        let base = wisp_sync::remote_relay_base(&public_url)?;
        ensure!(
            base.username().is_empty() && base.password().is_none(),
            "Public URL must not contain credentials"
        );
        let public_url = base.to_string();
        let raw_code = required("WISP_SERVER_CODE")?;
        ensure!(
            raw_code.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
            "WISP_SERVER_CODE must be 32 hexadecimal digits, optionally separated by hyphens"
        );
        let code = RemoteCode::parse(&raw_code)?;
        let provider_name = get("WISP_PROVIDER").unwrap_or_else(|| "openai".into());
        let kind = match provider_name.as_str() {
            "openai" => ProviderKind::OpenAiCompatible,
            "openai_responses" => ProviderKind::OpenAiResponses,
            "anthropic" => ProviderKind::Anthropic,
            _ => bail!("Server supports API-key providers: openai, openai_responses, anthropic"),
        };
        let default_url = if provider_name == "anthropic" {
            "https://api.anthropic.com"
        } else {
            "https://api.openai.com/v1"
        };
        let url = get("WISP_API_URL").unwrap_or_else(|| default_url.into());
        let model = required("WISP_MODEL")?;
        // Runtime injection for unattended deployments; never save this in SQLite.
        let mut provider = ProviderConfig::openai(&url, required("WISP_API_KEY")?, &model);
        provider.kind = kind;
        provider.reasoning_effort = get("WISP_REASONING_EFFORT");
        let catalog: catalog::Catalog = serde_json::from_str(include_str!(
            "../../../src-tauri/model_catalog.snapshot.json"
        ))?;
        let entry = catalog::lookup(&catalog, &provider_name, &url, &model);
        let positive = |key| -> Result<Option<u64>> {
            get(key)
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    let number = value
                        .parse::<u64>()
                        .with_context(|| format!("Invalid {key}"))?;
                    ensure!(number > 0, "{key} must be positive");
                    Ok(number)
                })
                .transpose()
        };
        let context = positive("WISP_MAX_CONTEXT")?;
        let output = positive("WISP_MAX_TOKENS")?;
        let (context, output) = match entry {
            Some(entry) => {
                let cap = entry.i.unwrap_or(entry.c).min(entry.c);
                (
                    context.unwrap_or(cap).min(cap),
                    output.unwrap_or(provider.max_tokens).min(entry.o),
                )
            }
            None => (
                context.context("Model is not in the exact-ID catalog; set WISP_MAX_CONTEXT")?,
                output.context("Model is not in the exact-ID catalog; set WISP_MAX_TOKENS")?,
            ),
        };
        ensure!(
            context > output && output > 0,
            "Context must exceed the output token budget"
        );
        provider.max_tokens = output;
        let data =
            PathBuf::from(get("WISP_SERVER_DATA").unwrap_or_else(|| "./wisp-server-data".into()));
        Ok(Self {
            workspace: get("WISP_SERVER_WORKSPACE")
                .map(PathBuf::from)
                .unwrap_or_else(|| data.join("workspace")),
            data,
            bind: get("WISP_SERVER_BIND")
                .unwrap_or_else(|| "127.0.0.1:8787".into())
                .parse()
                .context("Invalid WISP_SERVER_BIND")?,
            public_url,
            code,
            provider,
            max_context: usize::try_from(context)?,
            supports_vision: entry.is_some_and(|entry| entry.v),
            max_iterations: usize::try_from(positive("WISP_MAX_ITER")?.unwrap_or(200))?,
            skills: get("WISP_SKILLS_PATH")
                .map(|v| std::env::split_paths(&v).collect())
                .unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    fn base() -> HashMap<&'static str, String> {
        [
            ("WISP_SERVER_PUBLIC_URL", "https://science.example/wisp/"),
            ("WISP_SERVER_CODE", "0123456789abcdef0123456789abcdef"),
            ("WISP_MODEL", "test-model"),
            ("WISP_API_KEY", "synthetic"),
            ("WISP_MAX_CONTEXT", "32000"),
            ("WISP_MAX_TOKENS", "4000"),
        ]
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect()
    }
    #[test]
    fn config_requires_https_and_explicit_unknown_model_limits() {
        let mut values = base();
        let config = Config::parse(|k| values.get(k).cloned()).unwrap();
        assert_eq!(config.max_context, 32000);
        values.insert("WISP_SERVER_PUBLIC_URL", "http://192.168.1.1:8787".into());
        assert!(Config::parse(|k| values.get(k).cloned()).is_err());
        values.insert("WISP_SERVER_PUBLIC_URL", "http://localhost:8787".into());
        assert!(Config::parse(|k| values.get(k).cloned()).is_ok());
        values.remove("WISP_MAX_CONTEXT");
        assert!(Config::parse(|k| values.get(k).cloned()).is_err());
    }
    #[test]
    fn code_and_provider_fail_closed() {
        for (key, value) in [
            ("WISP_SERVER_CODE", "secret!"),
            ("WISP_PROVIDER", "typo"),
            ("WISP_MAX_ITER", "0"),
            ("WISP_API_KEY", ""),
        ] {
            let mut values = base();
            values.insert(key, value.into());
            assert!(Config::parse(|k| values.get(k).cloned()).is_err(), "{key}");
        }
    }
    #[test]
    fn empty_compose_overrides_use_catalog_caps() {
        let mut values = base();
        values.insert("WISP_MODEL", "gpt-4o".into());
        values.insert("WISP_MAX_CONTEXT", "".into());
        values.insert("WISP_MAX_TOKENS", "".into());
        let config = Config::parse(|k| values.get(k).cloned()).unwrap();
        assert!(config.max_context > config.provider.max_tokens as usize);
        values.insert("WISP_MAX_CONTEXT", "99999999".into());
        values.insert("WISP_MAX_TOKENS", "99999999".into());
        let limited = Config::parse(|k| values.get(k).cloned()).unwrap();
        assert_eq!(limited.max_context, config.max_context);
        assert!(limited.provider.max_tokens < 99999999);
    }
}
