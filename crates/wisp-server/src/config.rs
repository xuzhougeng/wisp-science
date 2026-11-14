use anyhow::{bail, ensure, Context, Result};
use std::{net::SocketAddr, path::PathBuf};
use wisp_llm::{ProviderConfig, ProviderKind};
use wisp_sync::RemoteCode;

// Use the exact lookup and checked-in catalog also used by the desktop. No
// network fetch or Tauri build is required for the server.
#[allow(dead_code)]
#[path = "../../../src-tauri/src/model_catalog_shared.rs"]
mod catalog;

/// The model a turn runs on, with the exact-ID catalog limits already applied.
#[derive(Clone)]
pub struct ModelConfig {
    pub provider: ProviderConfig,
    pub max_context: usize,
    pub supports_vision: bool,
}

#[derive(Clone)]
pub struct Config {
    pub data: PathBuf,
    pub workspace: PathBuf,
    pub bind: SocketAddr,
    pub public_url: String,
    pub code: RemoteCode,
    /// Always set for the headless server. An embedding desktop may start
    /// without one and configure it later through its settings.
    pub model: Option<ModelConfig>,
    pub max_iterations: usize,
    pub skills: Vec<PathBuf>,
}

fn positive(get: &impl Fn(&str) -> Option<String>, key: &str) -> Result<Option<u64>> {
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
}

impl ModelConfig {
    /// Reads `WISP_PROVIDER`, `WISP_API_URL`, `WISP_MODEL`, `WISP_API_KEY`,
    /// `WISP_REASONING_EFFORT`, `WISP_MAX_CONTEXT` and `WISP_MAX_TOKENS` through
    /// `get`, so the environment and saved desktop settings share one validation.
    pub fn resolve(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let required = |name| {
            get(name)
                .filter(|v| !v.trim().is_empty())
                .with_context(|| format!("{name} is required"))
        };
        let provider_name = get("WISP_PROVIDER")
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "openai".into());
        let kind = match provider_name.as_str() {
            "openai" => ProviderKind::OpenAiCompatible,
            "openai_responses" => ProviderKind::OpenAiResponses,
            "anthropic" => ProviderKind::Anthropic,
            _ => bail!("Server supports API-key providers: openai, openai_responses, anthropic"),
        };
        let url = get("WISP_API_URL")
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| default_api_url(&provider_name).into());
        let model = required("WISP_MODEL")?;
        // Runtime injection for unattended deployments; never save this in SQLite.
        let mut provider = ProviderConfig::openai(&url, required("WISP_API_KEY")?, &model);
        provider.kind = kind;
        provider.reasoning_effort = get("WISP_REASONING_EFFORT").filter(|v| !v.trim().is_empty());
        let catalog: catalog::Catalog = serde_json::from_str(include_str!(
            "../../../src-tauri/model_catalog.snapshot.json"
        ))?;
        let entry = catalog::lookup(&catalog, &provider_name, &url, &model);
        let context = positive(&get, "WISP_MAX_CONTEXT")?;
        let output = positive(&get, "WISP_MAX_TOKENS")?;
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
        Ok(Self {
            provider,
            max_context: usize::try_from(context)?,
            supports_vision: entry.is_some_and(|entry| entry.v),
        })
    }
}

/// Endpoint used when a provider is chosen without an explicit URL.
pub fn default_api_url(provider: &str) -> &'static str {
    if provider == "anthropic" {
        "https://api.anthropic.com"
    } else {
        "https://api.openai.com/v1"
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::parse(|name| std::env::var(name).ok())
    }

    /// Embed the host in a local desktop without a listener or pairing setup.
    /// Provider configuration and exact model limits are shared with the server.
    pub fn local(data: PathBuf, workspace: PathBuf) -> Result<Self> {
        Self::parse_local(data, workspace, |name| std::env::var(name).ok())
    }

    fn parse_local(
        data: PathBuf,
        workspace: PathBuf,
        get: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        let code = RemoteCode::generate()?.display();
        // A desktop without WISP_MODEL starts unconfigured and uses its saved
        // settings. A model named in the environment must still be valid.
        let model = match get("WISP_MODEL").filter(|v| !v.trim().is_empty()) {
            Some(_) => Some(ModelConfig::resolve(&get)?),
            None => None,
        };
        let mut config = Self::parse_host(
            |name| match name {
                "WISP_SERVER_PUBLIC_URL" => Some("http://localhost/".into()),
                "WISP_SERVER_CODE" => Some(code.clone()),
                "WISP_SERVER_BIND" => Some("127.0.0.1:0".into()),
                "WISP_SERVER_DATA" | "WISP_SERVER_WORKSPACE" => None,
                _ => get(name),
            },
            model,
        )?;
        // Keep OS paths lossless (in particular, non-UTF-8 Unix paths).
        config.data = data;
        config.workspace = workspace;
        Ok(config)
    }

    fn parse(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let model = ModelConfig::resolve(&get)?;
        Self::parse_host(get, Some(model))
    }

    fn parse_host(
        get: impl Fn(&str) -> Option<String>,
        model: Option<ModelConfig>,
    ) -> Result<Self> {
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
            model,
            max_iterations: usize::try_from(positive(&get, "WISP_MAX_ITER")?.unwrap_or(200))?,
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
        assert_eq!(config.model.unwrap().max_context, 32000);
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
        let config = Config::parse(|k| values.get(k).cloned())
            .unwrap()
            .model
            .unwrap();
        assert!(config.max_context > config.provider.max_tokens as usize);
        values.insert("WISP_MAX_CONTEXT", "99999999".into());
        values.insert("WISP_MAX_TOKENS", "99999999".into());
        let limited = Config::parse(|k| values.get(k).cloned())
            .unwrap()
            .model
            .unwrap();
        assert_eq!(limited.max_context, config.max_context);
        assert!(limited.provider.max_tokens < 99999999);
    }

    #[test]
    fn local_host_needs_no_remote_configuration_and_ignores_server_paths() {
        let mut values = base();
        values.remove("WISP_SERVER_PUBLIC_URL");
        values.remove("WISP_SERVER_CODE");
        values.insert("WISP_SERVER_BIND", "invalid".into());
        values.insert("WISP_SERVER_DATA", "unrelated-server".into());
        let local = Config::parse_local("desktop-data".into(), "project".into(), |k| {
            values.get(k).cloned()
        })
        .unwrap();
        assert_eq!(local.data, PathBuf::from("desktop-data"));
        assert_eq!(local.workspace, PathBuf::from("project"));
        assert_eq!(local.bind.port(), 0);
        assert_eq!(local.model.unwrap().max_context, 32_000);
        values.remove("WISP_MAX_TOKENS");
        assert!(Config::parse_local("data".into(), "project".into(), |k| {
            values.get(k).cloned()
        })
        .is_err());
        // Without a model in the environment the desktop starts unconfigured;
        // the headless server still refuses to.
        values.remove("WISP_MODEL");
        values.remove("WISP_API_KEY");
        let unconfigured =
            Config::parse_local("data".into(), "project".into(), |k| values.get(k).cloned())
                .unwrap();
        assert!(unconfigured.model.is_none());
        let mut headless = base();
        headless.remove("WISP_MODEL");
        assert!(Config::parse(|k| headless.get(k).cloned()).is_err());
    }
}
