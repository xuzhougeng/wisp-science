//! Extensions domain: the Settings "Skills" and "Plugins" panes — list/search
//! state plus install/enable/remove handlers. `App` constructs
//! `ExtensionsState` with the root-owned install modal flag and locale, and
//! passes the handlers down to `SettingsView` as callbacks.

use super::*;

/// Owned pane state plus injected cross-domain wiring. `RwSignal` is `Copy`,
/// so the whole struct is `Copy` and can be captured by every handler closure.
#[derive(Clone, Copy)]
pub(crate) struct ExtensionsState {
    pub(crate) skills_list: RwSignal<Vec<SkillRow>>,
    pub(crate) skills_search: RwSignal<String>,
    pub(crate) skills_msg: RwSignal<Option<(bool, String)>>,
    pub(crate) skill_filter_tag: RwSignal<String>,
    pub(crate) plugins_list: RwSignal<Vec<PluginRow>>,
    pub(crate) plugins_msg: RwSignal<Option<(bool, String)>>,
    // Cross-domain signals injected by `App`. The install modal flag stays
    // root-owned so the window-level Escape stack can close it.
    pub(crate) plugin_install_open: RwSignal<bool>,
    pub(crate) locale: RwSignal<Locale>,
}

impl ExtensionsState {
    pub(crate) fn new(plugin_install_open: RwSignal<bool>, locale: RwSignal<Locale>) -> Self {
        Self {
            skills_list: create_rw_signal(Vec::<SkillRow>::new()),
            skills_search: create_rw_signal(String::new()),
            skills_msg: create_rw_signal(None::<(bool, String)>),
            skill_filter_tag: create_rw_signal(String::new()),
            plugins_list: create_rw_signal(Vec::<PluginRow>::new()),
            plugins_msg: create_rw_signal(None::<(bool, String)>),
            plugin_install_open,
            locale,
        }
    }

    pub(crate) fn refresh_skills(self) {
        let skills_list = self.skills_list;
        spawn_local(async move {
            let v = invoke("list_skills", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<SkillRow>>(v) {
                skills_list.set(rows);
            }
        });
    }

    pub(crate) fn reload_skills(self) {
        let Self {
            skills_list,
            skills_msg,
            locale,
            ..
        } = self;
        spawn_local(async move {
            match invoke_checked("reload_skills", JsValue::UNDEFINED).await {
                Ok(value) => match serde_wasm_bindgen::from_value::<Vec<SkillRow>>(value) {
                    Ok(rows) => {
                        let total = rows.len().to_string();
                        skills_list.set(rows);
                        skills_msg.set(Some((
                            true,
                            tf(locale.get(), "skills.reloaded", &[("total", &total)]),
                        )));
                    }
                    Err(error) => skills_msg.set(Some((false, error.to_string()))),
                },
                Err(error) => skills_msg.set(Some((
                    false,
                    localize_backend(locale.get(), &js_error_text(error)),
                ))),
            }
        });
    }

    pub(crate) fn install_skill_from(self, path: String) {
        let Self {
            skills_msg, locale, ..
        } = self;
        spawn_local(async move {
            let arg = to_value(&serde_json::json!({ "srcPath": path })).unwrap();
            match invoke_checked("install_skill", arg).await {
                Ok(_) => {
                    skills_msg.set(Some((true, t(locale.get(), "skills.installed").into())));
                    self.refresh_skills();
                }
                Err(err) => {
                    skills_msg.set(Some((
                        false,
                        localize_backend(locale.get(), &js_error_text(err)),
                    )));
                }
            }
        });
    }

    pub(crate) fn save_skill_tags(self, name: String, raw: String) {
        let tags = split_tags(&raw);
        spawn_local(async move {
            let _ = invoke_checked(
                "set_skill_tags",
                to_value(&serde_json::json!({ "name": name, "tags": tags })).unwrap(),
            )
            .await;
            self.refresh_skills();
        });
    }

    pub(crate) fn set_visible_skills_enabled(self, enabled: bool) {
        let skills_list = self.skills_list;
        let tag = self.skill_filter_tag.get();
        let query = self.skills_search.get();
        let names = skills_list
            .get()
            .into_iter()
            .filter(|s| !s.managed && skill_matches_filter(s, &tag, &query))
            .map(|s| s.name)
            .collect::<Vec<_>>();
        if names.is_empty() {
            return;
        }
        let names_for_update = names.clone();
        skills_list.update(|list| {
            for skill in list {
                if names_for_update.contains(&skill.name) {
                    skill.enabled = enabled;
                }
            }
        });
        spawn_local(async move {
            let _ = invoke_checked(
                "set_skills_enabled",
                to_value(&serde_json::json!({ "names": names, "enabled": enabled })).unwrap(),
            )
            .await;
            self.refresh_skills();
        });
    }

    pub(crate) fn refresh_plugins(self) {
        let plugins_list = self.plugins_list;
        spawn_local(async move {
            let value = invoke("list_plugins", JsValue::UNDEFINED).await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<PluginRow>>(value) {
                plugins_list.set(rows);
            }
        });
    }

    pub(crate) fn install_plugin_from(self, path: String, expected_sha256: Option<String>) {
        let Self {
            plugins_msg,
            plugin_install_open,
            locale,
            ..
        } = self;
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "srcPath": path,
                "expectedSha256": expected_sha256,
            }))
            .unwrap();
            match invoke_checked("install_plugin", args).await {
                Ok(_) => {
                    plugins_msg.set(Some((true, t(locale.get(), "plugins.installed").into())));
                    plugin_install_open.set(false);
                    self.refresh_plugins();
                }
                Err(error) => {
                    plugins_msg.set(Some((
                        false,
                        localize_backend(locale.get(), &js_error_text(error)),
                    )));
                    self.refresh_plugins();
                }
            }
        });
    }

    pub(crate) fn install_plugin_url(self, source_url: String, expected_sha256: String) {
        let Self {
            plugins_msg,
            plugin_install_open,
            locale,
            ..
        } = self;
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "sourceUrl": source_url,
                "expectedSha256": expected_sha256,
            }))
            .unwrap();
            match invoke_checked("install_plugin_url", args).await {
                Ok(_) => {
                    plugins_msg.set(Some((true, t(locale.get(), "plugins.installed").into())));
                    plugin_install_open.set(false);
                    self.refresh_plugins();
                }
                Err(error) => {
                    plugins_msg.set(Some((
                        false,
                        localize_backend(locale.get(), &js_error_text(error)),
                    )));
                    self.refresh_plugins();
                }
            }
        });
    }

    pub(crate) fn set_plugin_enabled(self, id: String, version: String, enabled: bool) {
        let Self {
            plugins_msg,
            locale,
            ..
        } = self;
        spawn_local(async move {
            let args = to_value(&serde_json::json!({
                "pluginId": id,
                "version": version,
                "enabled": enabled,
            }))
            .unwrap();
            match invoke_checked("set_plugin_enabled", args).await {
                Ok(_) => {
                    plugins_msg.set(None);
                    self.refresh_plugins();
                    self.refresh_skills();
                }
                Err(error) => {
                    plugins_msg.set(Some((
                        false,
                        localize_backend(locale.get(), &js_error_text(error)),
                    )));
                    self.refresh_plugins();
                }
            }
        });
    }

    pub(crate) fn remove_plugin(self, id: String, version: String) {
        let Self {
            plugins_msg,
            locale,
            ..
        } = self;
        spawn_local(async move {
            let args =
                to_value(&serde_json::json!({ "pluginId": id, "version": version })).unwrap();
            match invoke_checked("remove_plugin", args).await {
                Ok(_) => {
                    plugins_msg.set(None);
                    self.refresh_plugins();
                    self.refresh_skills();
                }
                Err(error) => plugins_msg.set(Some((
                    false,
                    localize_backend(locale.get(), &js_error_text(error)),
                ))),
            }
        });
    }
}
