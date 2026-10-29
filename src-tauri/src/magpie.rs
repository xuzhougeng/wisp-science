//! Desktop Magpie override. The ordinary model editor and keyring stay intact.
pub(crate) fn config(session_id: Option<&str>) -> Option<wisp_llm::ProviderConfig> {
    if cfg!(test) {
        return None;
    } // Unit tests never inspect the user's HOME.
    let mut cfg = wisp_llm::magpie::startup_config()?;
    if let Some(entry) = crate::model_catalog::lookup("openai", &cfg.base_url, &cfg.model) {
        cfg.max_tokens = cfg.max_tokens.min(entry.o);
    }
    if let Some(id) = session_id {
        cfg.session_id = id.into();
    }
    Some(cfg)
}

pub(crate) fn context_window() -> Option<u64> {
    let cfg = config(None)?;
    Some(
        crate::model_catalog::lookup("openai", &cfg.base_url, &cfg.model)
            .map(|entry| entry.c)
            .unwrap_or(crate::models::DEFAULT_CONTEXT_WINDOW),
    )
}
