//! Desktop Magpie override. The ordinary model editor and keyring stay intact.

/// The connected model with Magpie's facts first, then the catalog by exact
/// model id, then the usual defaults.
#[derive(Clone)]
pub(crate) struct Runtime {
    config: wisp_llm::ProviderConfig,
    context_window: u64,
    efforts: Vec<String>,
}

impl Runtime {
    pub(crate) fn new(connection: wisp_llm::magpie::Connection) -> Self {
        let wisp_llm::magpie::Connection { mut config, info } = connection;
        let entry = crate::model_catalog::lookup("openai", &config.base_url, &config.model);
        if info.output.is_none() {
            if let Some(output) = entry.map(|entry| entry.o).filter(|tokens| *tokens >= 16) {
                config.max_tokens = output;
            }
        }
        let efforts = if info.efforts.is_empty() {
            entry.map(|entry| entry.e.clone()).unwrap_or_default()
        } else {
            info.efforts
        };
        config.reasoning_effort = wisp_llm::magpie::default_effort(&efforts).map(str::to_string);
        let context_window = info
            .context
            .or(entry.map(|entry| entry.c))
            .unwrap_or(crate::models::DEFAULT_CONTEXT_WINDOW);
        Self {
            config,
            context_window,
            efforts,
        }
    }
}

#[cfg(test)]
thread_local! {
    static TEST_RUNTIME: std::cell::RefCell<Option<Runtime>> = const { std::cell::RefCell::new(None) };
}

/// Unit tests never inspect the user's HOME; they install a fixture instead.
#[cfg(test)]
pub(crate) fn set_test_runtime(runtime: Option<Runtime>) {
    TEST_RUNTIME.with(|slot| *slot.borrow_mut() = runtime);
}

fn runtime() -> Option<Runtime> {
    #[cfg(test)]
    return TEST_RUNTIME.with(|slot| slot.borrow().clone());
    #[cfg(not(test))]
    {
        static RUNTIME: std::sync::OnceLock<Option<Runtime>> = std::sync::OnceLock::new();
        RUNTIME
            .get_or_init(|| wisp_llm::magpie::startup_connection().map(Runtime::new))
            .clone()
    }
}

pub(crate) fn config(session_id: Option<&str>) -> Option<wisp_llm::ProviderConfig> {
    let mut cfg = runtime()?.config;
    if let Some(id) = session_id {
        cfg.session_id = id.into();
    }
    Some(cfg)
}

pub(crate) fn context_window() -> Option<u64> {
    Some(runtime()?.context_window)
}

/// Effort for a request: the caller's value when the connected model supports
/// it, otherwise the model default. None leaves the provider default.
pub(crate) fn reasoning_effort(requested: &str) -> Option<String> {
    wisp_llm::magpie::resolve_effort(&runtime()?.efforts, requested)
}

pub(crate) fn supports_effort(effort: &str) -> bool {
    runtime().is_some_and(|runtime| runtime.efforts.iter().any(|known| known == effort))
}

/// What the composer needs to offer the connected model's effort levels.
/// Never carries the gateway URL or key.
#[derive(serde::Serialize)]
pub(crate) struct MagpieModelDto {
    model: String,
    context_window: u64,
    max_tokens: u64,
    efforts: Vec<String>,
    default_effort: String,
}

#[tauri::command]
pub(crate) fn magpie_model_info() -> Option<MagpieModelDto> {
    let runtime = runtime()?;
    Some(MagpieModelDto {
        model: runtime.config.model,
        context_window: runtime.context_window,
        max_tokens: runtime.config.max_tokens,
        default_effort: runtime.config.reasoning_effort.unwrap_or_default(),
        efforts: runtime.efforts,
    })
}
