use super::*;

pub(crate) fn normalize_settings_mut(cfg: &mut Settings) {
    cfg.provider = provider_value(&cfg.provider).into();
    cfg.api_url = cfg.api_url.trim().into();
    cfg.model = cfg.model.trim().into();
    cfg.sync_backend = if cfg.sync_backend == "folder" {
        "folder".into()
    } else {
        "relay".into()
    };
    cfg.sync_relay_url = cfg.sync_relay_url.trim().into();
    cfg.sync_folder = cfg.sync_folder.trim().into();
    cfg.auto_continue_limit = cfg.auto_continue_limit.max(1);
}

pub(crate) fn normalized_settings(mut cfg: Settings) -> Settings {
    normalize_settings_mut(&mut cfg);
    cfg
}

pub(crate) fn project_sync_backend_configured(cfg: &Settings) -> bool {
    match cfg.sync_backend.as_str() {
        "folder" => !cfg.sync_folder.trim().is_empty(),
        "relay" => {
            !cfg.sync_relay_url.trim().is_empty()
                && (cfg.has_sync_relay_token || !cfg.sync_relay_token.trim().is_empty())
        }
        _ => false,
    }
}

#[cfg(test)]
mod project_sync_backend_tests {
    use super::{project_sync_backend_configured, Settings};

    #[test]
    fn requires_a_complete_configuration_for_the_selected_backend() {
        let mut settings = Settings::default();
        assert!(!project_sync_backend_configured(&settings));

        settings.sync_relay_url = "https://relay.example.test".into();
        assert!(!project_sync_backend_configured(&settings));

        settings.has_sync_relay_token = true;
        assert!(project_sync_backend_configured(&settings));

        settings.sync_backend = "folder".into();
        assert!(!project_sync_backend_configured(&settings));

        settings.sync_folder = "C:\\Wisp Sync".into();
        assert!(project_sync_backend_configured(&settings));
    }
}

#[cfg(test)]
mod provider_form_tests {
    use super::{
        apply_base_url_suggestions, endpoint_has_stored_key, new_model_form,
        provider_entries_are_pristine, suggested_base_url_models, ModelProfile,
        DEEPSEEK_FLASH_MODEL, DEEPSEEK_PRO_MODEL,
    };

    fn profile(url: &str, has_key: bool) -> ModelProfile {
        ModelProfile {
            id: "m1".into(),
            label: "m".into(),
            provider: "openai".into(),
            api_url: url.into(),
            endpoint_suffix: String::new(),
            model: "x".into(),
            has_api_key: has_key,
            active: false,
            max_tokens: 0,
            context_window: 128_000,
            reasoning_effort: String::new(),
            service_tier: String::new(),
            supports_vision: false,
            use_for_vision: false,
            use_for_image_generation: false,
            image_size: String::new(),
            image_quality: String::new(),
            image_aspect_ratio: String::new(),
            image_resolution: String::new(),
            use_for_video_generation: false,
            video_duration_secs: None,
            video_aspect_ratio: None,
            video_resolution: None,
        }
    }

    #[test]
    fn deepseek_add_form_starts_with_flash_and_pro() {
        let form = new_model_form();
        let models: Vec<_> = form
            .entries
            .iter()
            .map(|entry| entry.model.as_str())
            .collect();
        assert_eq!(models, [DEEPSEEK_FLASH_MODEL, DEEPSEEK_PRO_MODEL]);
    }

    #[test]
    fn xai_base_url_suggests_chat_and_imagine_image() {
        let models: Vec<_> = suggested_base_url_models("https://api.x.ai/v1")
            .into_iter()
            .map(|entry| (entry.provider, entry.model, entry.use_for_image_generation))
            .collect();
        assert_eq!(
            models,
            vec![
                ("openai".into(), "grok-4.6".into(), false),
                ("openai".into(), "grok-imagine-image-2.0".into(), true),
            ]
        );
    }

    #[test]
    fn openai_base_url_suggests_responses_chat_and_image_models() {
        let models: Vec<_> = suggested_base_url_models("https://api.openai.com/v1")
            .into_iter()
            .map(|entry| (entry.provider, entry.model, entry.use_for_image_generation))
            .collect();
        assert_eq!(
            models,
            vec![
                ("openai_responses".into(), "gpt-5.5".into(), false),
                ("openai_responses".into(), "gpt-image-2".into(), true),
            ]
        );
    }

    #[test]
    fn changing_url_refreshes_only_pristine_suggestions() {
        let mut form = new_model_form();
        assert!(provider_entries_are_pristine(&form));
        apply_base_url_suggestions(&mut form, "https://api.kimi.com/coding/v1");
        assert_eq!(form.entries[0].model, "kimi-coding");
        form.entries[0].model = "k3-256k".into();
        let before = form.entries.clone();
        // User edited a row — a later URL tweak must not wipe it.
        let mut edited = form.clone();
        edited.api_url = "https://api.kimi.com/coding/v1/".into();
        assert!(!provider_entries_are_pristine(&edited));
        assert_eq!(edited.entries[0].model, before[0].model);
    }

    #[test]
    fn stored_key_matches_the_same_endpoint_only() {
        let models = vec![profile("https://api.deepseek.com", true)];
        assert!(endpoint_has_stored_key(
            &models,
            "https://api.deepseek.com/v1"
        ));
        assert!(!endpoint_has_stored_key(
            &models,
            "https://api.openai.com/v1"
        ));
    }
}

pub(crate) fn settings_required_error_key(cfg: &Settings, key: &str) -> Option<&'static str> {
    if cfg.api_url.trim().is_empty() {
        return Some("err.api_url_required");
    }
    if cfg.model.trim().is_empty() {
        return Some("err.model_required");
    }
    let has_new_key = !key.trim().is_empty();
    if !cfg.has_api_key && !has_new_key {
        return Some("err.api_key_required");
    }
    None
}

/// Single source of truth for `invoke` argument payloads.
///
/// Tauri v2 deserializes command arguments from JS **camelCase** keys onto the
/// Rust **snake_case** parameters. A snake_case key (`session_id`) never binds:
/// an `Option` param silently becomes `None`, which once made every send fork a
/// brand-new conversation instead of continuing the active one. Keep every
/// multi-word key camelCase here; `tauri_args_tests` pins them.
pub(crate) mod tauri_args {
    use serde_json::{json, Value};

    pub fn stop_agent(session_id: &Option<String>) -> Value {
        json!({ "sessionId": session_id })
    }
    pub fn review_session(session_id: &Option<String>) -> Value {
        json!({ "sessionId": session_id })
    }
    pub fn branch_session(
        session_id: &Option<String>,
        title: Option<&str>,
        user_index: Option<usize>,
        checkpoint_kind: Option<&str>,
    ) -> Value {
        let mut payload = json!({ "sessionId": session_id });
        if let Some(title) = title.map(str::trim).filter(|s| !s.is_empty()) {
            payload["title"] = json!(title);
        }
        if let Some(user_index) = user_index {
            payload["userIndex"] = json!(user_index);
        }
        if let Some(checkpoint_kind) = checkpoint_kind {
            payload["checkpointKind"] = json!(checkpoint_kind);
        }
        payload
    }
    pub fn start_exploration(
        source_frame_id: &str,
        turn_index: Option<usize>,
        name: &str,
    ) -> Value {
        json!({
            "sourceFrameId": source_frame_id,
            "turnIndex": turn_index,
            "name": name,
        })
    }
    pub fn exploration(exploration_id: &str) -> Value {
        json!({ "explorationId": exploration_id })
    }
    pub fn promote_exploration(exploration_id: &str, expected_guard_hash: &str) -> Value {
        json!({
            "explorationId": exploration_id,
            "expectedGuardHash": expected_guard_hash,
        })
    }
    pub fn rewind_session(session_id: &Option<String>, user_index: usize) -> Value {
        json!({ "sessionId": session_id, "userIndex": user_index })
    }
    pub fn turn_undo(session_id: &str, user_index: usize) -> Value {
        json!({ "sessionId": session_id, "userIndex": user_index })
    }
    pub fn confirm_response(
        session_id: &str,
        approved: bool,
        feedback: Option<&str>,
        scope: Option<&str>,
    ) -> Value {
        let mut payload = json!({ "sessionId": session_id, "approved": approved });
        if let Some(feedback) = feedback.map(str::trim).filter(|s| !s.is_empty()) {
            payload["feedback"] = json!(feedback);
        }
        if let Some(scope) = scope.map(str::trim).filter(|s| !s.is_empty()) {
            payload["scope"] = json!(scope);
        }
        payload
    }
    pub fn read_file(path: &str, max_bytes: Option<u64>) -> Value {
        match max_bytes {
            Some(n) => json!({ "path": path, "maxBytes": n }),
            None => json!({ "path": path }),
        }
    }
}

#[cfg(test)]
mod tauri_args_tests {
    use super::*;

    // Guard the exact regression: `session_id` must reach the backend as the
    // camelCase `sessionId`, or `send_message` starts a new conversation.
    #[test]
    fn send_message_args_serialize_camel_case() {
        let v = serde_json::to_value(SendMessageArgs {
            session_id: Some("frame-1".into()),
            message: "hi".into(),
            attachments: vec!["a.png".into()],
            references: vec![],
            resume: false,
            acp_agent_id: None,
            guide: None,
            replace: None,
        })
        .unwrap();
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["message"], "hi");
        assert_eq!(v["attachments"][0], "a.png");
        assert!(
            v.get("session_id").is_none(),
            "snake_case key would bind to None on the backend"
        );
    }

    #[test]
    fn session_command_args_use_camel_case_keys() {
        let sid = Some("frame-1".to_string());

        let v = tauri_args::stop_agent(&sid);
        assert_eq!(v["sessionId"], "frame-1");
        assert!(v.get("session_id").is_none());

        let v = tauri_args::review_session(&sid);
        assert_eq!(v["sessionId"], "frame-1");

        let v = tauri_args::branch_session(&sid, Some("fork here"), Some(2), Some("before_user"));
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["title"], "fork here");
        assert_eq!(v["userIndex"], 2);
        assert_eq!(v["checkpointKind"], "before_user");
        assert!(v.get("session_id").is_none());
        assert!(v.get("user_index").is_none());

        let v = tauri_args::start_exploration("frame-1", Some(2), "Try A");
        assert_eq!(v["sourceFrameId"], "frame-1");
        assert_eq!(v["turnIndex"], 2);
        assert_eq!(v["name"], "Try A");
        let v = tauri_args::exploration("exploration-1");
        assert_eq!(v["explorationId"], "exploration-1");
        let v = tauri_args::promote_exploration("exploration-1", "guard-1");
        assert_eq!(v["explorationId"], "exploration-1");
        assert_eq!(v["expectedGuardHash"], "guard-1");

        let v = tauri_args::rewind_session(&sid, 3);
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["userIndex"], 3);
        assert!(v.get("user_index").is_none());

        let v = tauri_args::turn_undo("frame-1", 4);
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["userIndex"], 4);
        assert!(v.get("session_id").is_none());
        assert!(v.get("user_index").is_none());

        let v = tauri_args::confirm_response("frame-1", true, None, Some("once"));
        assert_eq!(v["sessionId"], "frame-1");
        assert_eq!(v["approved"], true);
        assert_eq!(v["scope"], "once");
        assert!(v.get("feedback").is_none());

        let v = tauri_args::confirm_response("frame-1", false, Some("split the plan"), None);
        assert_eq!(v["feedback"], "split the plan");
        assert!(v.get("scope").is_none());

        let v = tauri_args::read_file("a.txt", Some(1024));
        assert_eq!(v["path"], "a.txt");
        assert_eq!(v["maxBytes"], 1024);
        assert!(v.get("max_bytes").is_none());
    }

    // The agent is told to emit absolute paths, so a clicked file link must reach
    // the backend intact. Stripping the leading slash turned `/Users/…/fig.png`
    // into a bad root-relative path that 404'd on click (#12).
    #[test]
    fn normalize_path_keeps_absolute_paths() {
        assert_eq!(
            normalize_path("/Users/x/proj/results/fig.png"),
            "/Users/x/proj/results/fig.png"
        );
        assert_eq!(normalize_path("C:\\proj\\out.csv"), "C:\\proj\\out.csv");
        // Redundant current-dir prefixes are still trimmed; relative stays relative.
        assert_eq!(normalize_path("./results/fig.png"), "results/fig.png");
        assert_eq!(normalize_path(".\\results\\fig.png"), "results\\fig.png");
        assert_eq!(normalize_path("results/fig.png"), "results/fig.png");
        assert_eq!(normalize_path("  /a/b.txt  "), "/a/b.txt");
        assert_eq!(
            normalize_path("figures/panel_I_heatmap_4genes_median.png/.pdf"),
            "figures/panel_I_heatmap_4genes_median.png"
        );
        assert_eq!(
            normalize_path("./figures/plot.JPG/.PDF"),
            "figures/plot.JPG"
        );
        assert_eq!(
            normalize_path("C:\\proj\\fig.png\\.pdf"),
            "C:\\proj\\fig.png"
        );
    }

    #[test]
    fn collect_artifacts_normalizes_image_pdf_shorthand() {
        let items = vec![ChatItem::FileChanged(
            "figures/panel_I_heatmap_4genes_median.png/.pdf".into(),
        )];
        let arts = collect_artifacts(&items, Locale::En, &mut ProtoCache::new());
        let a = arts
            .iter()
            .find(|a| a.name == "panel_I_heatmap_4genes_median.png")
            .unwrap();
        assert_eq!(a.kind, "image");
        match &a.data {
            PreviewData::File { path, kind } => {
                assert_eq!(path, "figures/panel_I_heatmap_4genes_median.png");
                assert_eq!(kind, "image");
            }
            _ => panic!("expected file artifact"),
        }
    }
}

pub(crate) fn split_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn join_tags(tags: &[String]) -> String {
    tags.join(", ")
}

pub(crate) fn skill_matches_filter(skill: &SkillRow, tag: &str, query: &str) -> bool {
    let tag_match = match tag {
        "" => true,
        "__untagged" => skill.tags.is_empty(),
        "__enabled" => skill.enabled,
        "__disabled" => !skill.enabled,
        t => skill.tags.iter().any(|s| s == t),
    };
    let q = query.trim().to_ascii_lowercase();
    tag_match
        && (q.is_empty()
            || skill.name.to_ascii_lowercase().contains(&q)
            || skill.description.to_ascii_lowercase().contains(&q))
}

#[cfg(test)]
mod skill_filter_tests {
    use super::*;

    fn skill(name: &str, enabled: bool, tags: &[&str]) -> SkillRow {
        SkillRow {
            name: name.into(),
            description: "Scientific workflow".into(),
            tags: tags.iter().map(|tag| (*tag).into()).collect(),
            scope: "bundled".into(),
            enabled,
            builtin: true,
            managed: false,
            managed_by: None,
            dir: String::new(),
        }
    }

    #[test]
    fn skill_status_filters_compose_with_search() {
        let enabled = skill("remote-compute", true, &["compute"]);
        let disabled = skill("literature-review", false, &[]);

        assert!(skill_matches_filter(&enabled, "__enabled", "remote"));
        assert!(!skill_matches_filter(&enabled, "__disabled", ""));
        assert!(skill_matches_filter(&disabled, "__disabled", "workflow"));
        assert!(!skill_matches_filter(&disabled, "__enabled", ""));
    }
}

pub(crate) fn refresh_capabilities(caps: RwSignal<Option<Capabilities>>) {
    spawn_local(async move {
        let v = invoke("get_capabilities", JsValue::UNDEFINED).await;
        if let Ok(data) = serde_wasm_bindgen::from_value::<Capabilities>(v) {
            caps.set(Some(data));
        }
    });
}

/// Decide how `get_acp_session_agent` should update the picker selection.
///
/// Returning `None` means "leave the current selection alone" — needed when the
/// first ACP turn is still binding the session and the backend still reports
/// `None` (otherwise the picker snaps back to the HTTP model mid-send).
pub(crate) fn acp_agent_selection_after_fetch(
    fetched: Option<String>,
    session_id: &str,
    pending: &HashMap<String, usize>,
    running: &HashSet<String>,
    provisional: Option<&(String, String)>,
) -> Option<Option<String>> {
    match fetched {
        Some(id) => Some(Some(id)),
        None if provisional.is_some_and(|(frame_id, _)| frame_id == session_id) => {
            Some(provisional.map(|(_, agent_id)| agent_id.clone()))
        }
        None if pending.contains_key(session_id) || running.contains(session_id) => None,
        None => Some(None),
    }
}

/// Fold a `CurrentModeUpdate` payload into the stored SessionModeState.
///
/// The update only carries `currentModeId`, so when we already hold the initial
/// `SessionModeState` we keep its `availableModes` (which the mode picker needs)
/// and only swap the current id. With no prior object, the payload stands alone.
pub(crate) fn merge_current_mode(
    existing: Option<&serde_json::Value>,
    payload: serde_json::Value,
) -> serde_json::Value {
    if let (Some(serde_json::Value::Object(existing)), Some(id)) =
        (existing, payload.get("currentModeId"))
    {
        let mut merged = existing.clone();
        merged.insert("currentModeId".into(), id.clone());
        return serde_json::Value::Object(merged);
    }
    payload
}

#[cfg(test)]
mod merge_current_mode_tests {
    use super::merge_current_mode;
    use serde_json::json;

    #[test]
    fn preserves_available_modes_on_update() {
        let existing = json!({
            "currentModeId": "full-access",
            "availableModes": [{"id": "agent", "name": "Agent"}, {"id": "full-access", "name": "Full Access"}],
        });
        let merged = merge_current_mode(Some(&existing), json!({ "currentModeId": "agent" }));
        assert_eq!(merged["currentModeId"], json!("agent"));
        assert_eq!(merged["availableModes"], existing["availableModes"]);
    }

    #[test]
    fn falls_back_to_payload_without_prior_state() {
        let merged = merge_current_mode(None, json!({ "currentModeId": "agent" }));
        assert_eq!(merged, json!({ "currentModeId": "agent" }));
    }
}

#[cfg(test)]
mod acp_agent_selection_tests {
    use super::acp_agent_selection_after_fetch;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn applies_bound_agent() {
        let pending = HashMap::new();
        let running = HashSet::new();
        assert_eq!(
            acp_agent_selection_after_fetch(Some("agent-1".into()), "s1", &pending, &running, None),
            Some(Some("agent-1".into()))
        );
    }

    #[test]
    fn preserves_selection_while_first_turn_pending() {
        let mut pending = HashMap::new();
        pending.insert("s1".into(), 1);
        let running = HashSet::new();
        assert_eq!(
            acp_agent_selection_after_fetch(None, "s1", &pending, &running, None),
            None
        );
    }

    #[test]
    fn preserves_provisional_agent_on_a_fresh_session() {
        let pending = HashMap::new();
        let running = HashSet::new();
        let provisional = ("s1".into(), "agent-1".into());
        assert_eq!(
            acp_agent_selection_after_fetch(None, "s1", &pending, &running, Some(&provisional)),
            Some(Some("agent-1".into()))
        );
    }

    #[test]
    fn clears_when_session_has_no_binding() {
        let pending = HashMap::new();
        let running = HashSet::new();
        assert_eq!(
            acp_agent_selection_after_fetch(None, "s1", &pending, &running, None),
            Some(None)
        );
    }
}

pub(crate) fn reviewer_backend_key(reviewer: &Specialist) -> String {
    match &reviewer.review_backend {
        Some(ReviewBackendConfig::FollowSession) => "follow_session".into(),
        Some(ReviewBackendConfig::AcpAgent { profile_id }) => format!("acp:{profile_id}"),
        Some(ReviewBackendConfig::HttpModel { profile_id }) => format!("http:{profile_id}"),
        None => format!("http:{}", reviewer.model_id),
    }
}

pub(crate) fn set_reviewer_backend(reviewer: &mut Specialist, key: &str) {
    if key == "follow_session" {
        reviewer.review_backend = Some(ReviewBackendConfig::follow_session());
    } else if let Some(profile_id) = key.strip_prefix("acp:") {
        reviewer.review_backend = Some(ReviewBackendConfig::acp(profile_id));
    } else {
        let profile_id = key.strip_prefix("http:").unwrap_or(key);
        reviewer.model_id = profile_id.to_string();
        reviewer.review_backend = Some(ReviewBackendConfig::http(profile_id));
    }
}

pub(crate) fn reviewer_backend_label(
    reviewer: &Specialist,
    models: &[ModelProfile],
    acp_agents: &[AcpAgentProfile],
    follow_session_label: &str,
    missing_acp_label: &str,
) -> Option<String> {
    match &reviewer.review_backend {
        Some(ReviewBackendConfig::FollowSession) => Some(follow_session_label.into()),
        Some(ReviewBackendConfig::AcpAgent { profile_id }) => Some(
            acp_agents
                .iter()
                .find(|profile| profile.id == *profile_id)
                .map(|profile| format!("{} · ACP", profile.label))
                .unwrap_or_else(|| format!("{missing_acp_label} · {profile_id}")),
        ),
        Some(ReviewBackendConfig::HttpModel { profile_id }) => {
            if profile_id.is_empty() {
                None
            } else {
                models
                    .iter()
                    .find(|profile| profile.id == *profile_id)
                    .map(|profile| profile.label.clone())
            }
        }
        None => {
            if reviewer.model_id.is_empty() {
                None
            } else {
                models
                    .iter()
                    .find(|profile| profile.id == reviewer.model_id)
                    .map(|profile| profile.label.clone())
            }
        }
    }
}

pub(crate) fn reviewer_missing_acp_profile_id(
    reviewer: &Specialist,
    acp_agents: &[AcpAgentProfile],
) -> Option<String> {
    let Some(ReviewBackendConfig::AcpAgent { profile_id }) = &reviewer.review_backend else {
        return None;
    };
    (!acp_agents.iter().any(|profile| profile.id == *profile_id)).then(|| profile_id.clone())
}

#[cfg(test)]
mod review_tests {
    use super::{
        reviewer_backend_key, reviewer_backend_label, reviewer_missing_acp_profile_id,
        set_reviewer_backend, upsert_review,
    };
    use crate::dto::{AcpAgentProfile, ChatItem, ReviewBackendConfig, ReviewReport, Specialist};

    fn report(id: &str, summary: &str) -> ReviewReport {
        ReviewReport {
            id: id.into(),
            summary: summary.into(),
            findings: vec![],
            reviewer_model: "review-model".into(),
            reviewer_effort: String::new(),
            reviewer_backend: "http_model".into(),
            review_status: "passed".into(),
            evidence_coverage: 100,
            coverage_gaps: vec![],
        }
    }

    #[test]
    fn follow_up_review_replaces_the_original_card() {
        let mut items = vec![ChatItem::Assistant {
            text: "answer".into(),
            model: None,
            resources: Vec::new(),
        }];
        upsert_review(&mut items, report("r1", "first"));
        upsert_review(&mut items, report("r1", "verified"));

        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[1],
            ChatItem::Review(report) if report.summary == "verified"
        ));
    }

    #[test]
    fn reviewer_backend_keys_roundtrip_http_acp_and_follow_session() {
        let mut reviewer = Specialist {
            id: "reviewer".into(),
            name: "Reviewer".into(),
            icon: String::new(),
            color: String::new(),
            description: String::new(),
            instructions: String::new(),
            model_id: String::new(),
            review_backend: None,
            skills: Some(vec![]),
            connectors: Some(vec![]),
            builtin: true,
        };

        set_reviewer_backend(&mut reviewer, "acp:codex");
        assert_eq!(reviewer_backend_key(&reviewer), "acp:codex");
        assert_eq!(
            reviewer.review_backend,
            Some(ReviewBackendConfig::acp("codex"))
        );

        set_reviewer_backend(&mut reviewer, "http:review-model");
        assert_eq!(reviewer_backend_key(&reviewer), "http:review-model");
        assert_eq!(reviewer.model_id, "review-model");

        set_reviewer_backend(&mut reviewer, "follow_session");
        assert_eq!(reviewer_backend_key(&reviewer), "follow_session");
    }

    #[test]
    fn missing_acp_reviewer_stays_visible_instead_of_looking_like_http() {
        let mut reviewer = Specialist {
            id: "reviewer".into(),
            name: "Reviewer".into(),
            icon: String::new(),
            color: String::new(),
            description: String::new(),
            instructions: String::new(),
            model_id: String::new(),
            review_backend: None,
            skills: Some(vec![]),
            connectors: Some(vec![]),
            builtin: true,
        };
        set_reviewer_backend(&mut reviewer, "acp:deleted-profile");
        let agents = vec![AcpAgentProfile {
            id: "other".into(),
            label: "Other ACP".into(),
            command: "other-acp".into(),
            args: vec![],
        }];

        assert_eq!(
            reviewer_missing_acp_profile_id(&reviewer, &agents).as_deref(),
            Some("deleted-profile")
        );
        assert_eq!(
            reviewer_backend_label(
                &reviewer,
                &[],
                &agents,
                "Follow session backend",
                "Missing ACP Agent",
            )
            .as_deref(),
            Some("Missing ACP Agent · deleted-profile")
        );
    }
}

pub(crate) fn profile_to_form(m: &ModelProfile) -> ModelForm {
    ModelForm {
        id: Some(m.id.clone()),
        label: m.label.clone(),
        provider: m.provider.clone(),
        api_url: m.api_url.clone(),
        endpoint_suffix: m.endpoint_suffix.clone(),
        model: m.model.clone(),
        max_tokens: if m.max_tokens >= 16 {
            m.max_tokens
        } else {
            8192
        },
        context_window: if m.context_window >= 4_096 {
            m.context_window
        } else {
            128_000
        },
        reasoning_effort: m.reasoning_effort.clone(),
        service_tier: m.service_tier.clone(),
        supports_vision: m.supports_vision,
        use_for_vision: m.use_for_vision,
        use_for_image_generation: m.use_for_image_generation,
        image_size: m.image_size.clone(),
        image_quality: m.image_quality.clone(),
        image_aspect_ratio: m.image_aspect_ratio.clone(),
        image_resolution: m.image_resolution.clone(),
        use_for_video_generation: m.use_for_video_generation,
        video_duration_secs: m.video_duration_secs,
        video_aspect_ratio: m.video_aspect_ratio.clone(),
        video_resolution: m.video_resolution.clone(),
        entries: Vec::new(),
    }
}

fn next_model_row_id() -> u64 {
    use std::cell::Cell;
    thread_local! {
        static NEXT: Cell<u64> = const { Cell::new(1) };
    }
    NEXT.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    })
}

pub(crate) fn model_form_entry(
    provider: &str,
    model: &str,
    endpoint_suffix: &str,
    image: bool,
) -> ModelFormEntry {
    let image = image || crate::dto::is_image_generation_model(model);
    let video = !image && crate::dto::is_video_generation_model(model);
    ModelFormEntry {
        row_id: next_model_row_id(),
        provider: provider_value(provider).into(),
        endpoint_suffix: endpoint_suffix.into(),
        label: String::new(),
        model: model.into(),
        supports_vision: false,
        use_for_vision: false,
        use_for_image_generation: image,
        use_for_video_generation: video,
    }
}

pub(crate) fn suggested_base_url_models(api_url: &str) -> Vec<ModelFormEntry> {
    let host = normalize_endpoint(api_url).to_ascii_lowercase();
    match host.as_str() {
        host if host.contains("api.anthropic.com") => {
            vec![model_form_entry("anthropic", "claude-sonnet-5", "", false)]
        }
        host if host.contains("api.openai.com") => vec![
            model_form_entry("openai_responses", "gpt-5.5", "", false),
            model_form_entry("openai_responses", "gpt-image-2", "", true),
        ],
        host if host.contains("api.x.ai") => vec![
            model_form_entry("openai", "grok-4.6", "", false),
            model_form_entry("openai", "grok-imagine-image-2.0", "", true),
        ],
        host if host.contains("deepseek.com") || host.is_empty() => vec![
            model_form_entry("openai", DEEPSEEK_FLASH_MODEL, "", false),
            model_form_entry("openai", DEEPSEEK_PRO_MODEL, "", false),
        ],
        host if host.contains("api.kimi.com") && host.contains("/coding") => {
            vec![model_form_entry("openai", "kimi-coding", "", false)]
        }
        host if host.contains("moonshot") || host.contains("api.kimi.com") => {
            vec![model_form_entry("openai", "kimi-k3", "", false)]
        }
        host if host.contains("bigmodel.cn") && host.contains("coding") => {
            vec![model_form_entry("openai", "glm-5.2", "", false)]
        }
        host if host.contains("bigmodel.cn") => {
            vec![model_form_entry("openai", "glm-5", "", false)]
        }
        _ => vec![model_form_entry("openai", "", "", false)],
    }
}

pub(crate) fn provider_entries_are_pristine(form: &ModelForm) -> bool {
    let suggested = suggested_base_url_models(&form.api_url);
    let current: Vec<_> = form
        .entries
        .iter()
        .map(|entry| {
            (
                provider_value(&entry.provider),
                entry.endpoint_suffix.trim(),
                entry.label.trim(),
                entry.model.trim(),
                entry.supports_vision,
                entry.use_for_vision,
                entry.use_for_image_generation,
            )
        })
        .collect();
    let expected: Vec<_> = suggested
        .iter()
        .map(|entry| {
            (
                provider_value(&entry.provider),
                entry.endpoint_suffix.trim(),
                entry.label.trim(),
                entry.model.trim(),
                entry.supports_vision,
                entry.use_for_vision,
                entry.use_for_image_generation,
            )
        })
        .collect();
    current == expected
}

pub(crate) fn apply_base_url_suggestions(form: &mut ModelForm, api_url: &str) {
    form.api_url = api_url.into();
    form.entries = suggested_base_url_models(&form.api_url);
}

pub(crate) fn endpoint_has_stored_key(models: &[ModelProfile], api_url: &str) -> bool {
    models
        .iter()
        .any(|profile| profile.has_api_key && same_endpoint(&profile.api_url, api_url))
}

pub(crate) fn sibling_profile_id<'a>(models: &'a [ModelProfile], api_url: &str) -> Option<&'a str> {
    models
        .iter()
        .find(|profile| profile.has_api_key && same_endpoint(&profile.api_url, api_url))
        .map(|profile| profile.id.as_str())
}

pub(crate) fn new_model_form() -> ModelForm {
    let (api_url, _) = provider_defaults("openai");
    ModelForm {
        provider: "openai".into(),
        api_url: api_url.into(),
        max_tokens: 8192,
        context_window: 128_000,
        entries: suggested_base_url_models(api_url),
        ..Default::default()
    }
}

pub(crate) fn new_acp_form() -> AcpAgentProfile {
    AcpAgentProfile {
        id: String::new(),
        label: String::new(),
        command: String::new(),
        args: Vec::new(),
    }
}

pub(crate) fn model_form_to_settings(form: &ModelForm, has_api_key: bool) -> Settings {
    let mut cfg = Settings::default();
    cfg.provider = provider_value(&form.provider).into();
    cfg.api_url = join_api_url(&form.api_url, &form.endpoint_suffix);
    cfg.model = form.model.trim().into();
    cfg.label = form.label.trim().into();
    cfg.has_api_key = has_api_key;
    cfg.max_tokens = form.max_tokens;
    cfg.reasoning_effort = form.reasoning_effort.clone();
    cfg.service_tier = form.service_tier.clone();
    cfg.supports_vision = form.supports_vision;
    cfg
}

pub(crate) fn settings_section_label(loc: Locale, section: &str) -> String {
    match section {
        "session" => t(loc, "settings.nav.session"),
        "appearance" => t(loc, "settings.nav.appearance"),
        "pet" => t(loc, "settings.nav.pet"),
        "environments" => t(loc, "settings.nav.environments"),
        "models" => t(loc, "settings.nav.models"),
        "quick-actions" => t(loc, "settings.nav.quick_actions"),
        "workflows" => t(loc, "settings.nav.workflows"),
        "specialists" => t(loc, "settings.nav.specialists"),
        "memory" => t(loc, "settings.nav.memory"),
        "skills" => t(loc, "settings.nav.skills"),
        "plugins" => t(loc, "settings.nav.plugins"),
        "browser" => t(loc, "settings.nav.browser"),
        "connections" => t(loc, "settings.nav.connections"),
        "channels" => t(loc, "settings.nav.channels"),
        "credentials" => t(loc, "settings.nav.credentials"),
        "permissions" => t(loc, "settings.nav.permissions"),
        "storage" => t(loc, "settings.nav.storage"),
        "usage" => t(loc, "settings.nav.usage"),
        _ => t(loc, "settings.title"),
    }
    .into()
}

#[cfg(test)]
mod settings_section_label_tests {
    use super::*;
    use crate::i18n::Locale;

    #[test]
    fn session_nav_has_its_own_label() {
        assert_eq!(settings_section_label(Locale::En, "session"), "Session");
        assert_eq!(settings_section_label(Locale::Zh, "session"), "对话");
    }
}

/// A field within a credential service group: (credential id, i18n label key,
/// whether to mask the value like a password).
pub(crate) struct CredField {
    pub(crate) id: &'static str,
    pub(crate) label_key: &'static str,
    pub(crate) secret: bool,
}

/// An official setup destination shown below a built-in credential group.
pub(crate) struct CredLink {
    pub(crate) label_key: &'static str,
    pub(crate) url: &'static str,
}

/// A credential service shown in Settings → Credentials: display name,
/// structured help, setup guidance, official links, and its fields. Mirrors
/// the backend `CREDENTIALS` registry in models.rs — keep ids in sync.
pub(crate) struct CredGroup {
    pub(crate) id: &'static str,
    pub(crate) name_key: &'static str,
    pub(crate) about_key: &'static str,
    pub(crate) configured_key: &'static str,
    pub(crate) unconfigured_key: &'static str,
    pub(crate) hint_key: &'static str,
    pub(crate) links: &'static [CredLink],
    pub(crate) fields: &'static [CredField],
}

pub(crate) const CRED_GROUPS: &[CredGroup] = &[
    CredGroup {
        id: "openalex",
        name_key: "cred.openalex.name",
        about_key: "cred.openalex.about",
        configured_key: "cred.openalex.configured",
        unconfigured_key: "cred.openalex.unconfigured",
        hint_key: "cred.openalex.hint",
        links: &[CredLink {
            label_key: "cred.openalex.link",
            url: "https://openalex.org/settings/api",
        }],
        fields: &[CredField {
            id: "openalex_api_key",
            label_key: "cred.openalex_api_key.label",
            secret: true,
        }],
    },
    CredGroup {
        id: "infinisynapse",
        name_key: "cred.infinisynapse.name",
        about_key: "cred.infinisynapse.about",
        configured_key: "cred.infinisynapse.configured",
        unconfigured_key: "cred.infinisynapse.unconfigured",
        hint_key: "cred.infinisynapse.hint",
        links: &[
            CredLink {
                label_key: "cred.infinisynapse.console_link",
                url: "https://app.infinisynapse.cn/tasks",
            },
            CredLink {
                label_key: "cred.infinisynapse.docs_link",
                url: "https://infinisynapse.cn",
            },
        ],
        fields: &[CredField {
            id: "infinisynapse_api_key",
            label_key: "cred.infinisynapse_api_key.label",
            secret: true,
        }],
    },
    CredGroup {
        id: "scimaster",
        name_key: "cred.scimaster.name",
        about_key: "cred.scimaster.about",
        configured_key: "cred.scimaster.configured",
        unconfigured_key: "cred.scimaster.unconfigured",
        hint_key: "cred.scimaster.hint",
        links: &[CredLink {
            label_key: "cred.scimaster.link",
            url: "https://scimaster.bohrium.com/vibe-write/home",
        }],
        fields: &[CredField {
            id: "scimaster_api_key",
            label_key: "cred.scimaster_api_key.label",
            secret: true,
        }],
    },
    CredGroup {
        id: "ncbi",
        name_key: "cred.ncbi.name",
        about_key: "cred.ncbi.about",
        configured_key: "cred.ncbi.configured",
        unconfigured_key: "cred.ncbi.unconfigured",
        hint_key: "cred.ncbi.hint",
        links: &[CredLink {
            label_key: "cred.ncbi.link",
            url: "https://www.ncbi.nlm.nih.gov/account/",
        }],
        fields: &[
            CredField {
                id: "ncbi_api_key",
                label_key: "cred.ncbi_api_key.label",
                secret: true,
            },
            CredField {
                id: "ncbi_email",
                label_key: "cred.ncbi_email.label",
                secret: false,
            },
        ],
    },
];

pub(crate) fn settings_subpage_label(
    loc: Locale,
    section: &str,
    model_form: Option<&ModelForm>,
    conn_form: Option<&ConnForm>,
    open_conn: Option<&str>,
    memory_selected: Option<&str>,
    specialist_form: Option<&Specialist>,
    acp_form: Option<&AcpAgentProfile>,
    channels_open: Option<&str>,
) -> Option<String> {
    match section {
        "models" => acp_form
            .map(|f| {
                if f.id.is_empty() {
                    t(loc, "models.add_acp").into()
                } else {
                    t(loc, "models.edit_acp").into()
                }
            })
            .or_else(|| {
                model_form.map(|f| {
                    if f.id.is_some() {
                        t(loc, "models.edit").into()
                    } else {
                        t(loc, "models.add").into()
                    }
                })
            }),
        "specialists" => specialist_form.map(|s| {
            if s.id.is_empty() {
                t(loc, "specialists.add")
            } else {
                s.name.clone()
            }
        }),
        "connections" => conn_form
            .map(|f| {
                if f.id.is_some() {
                    t(loc, "conn.edit").into()
                } else {
                    t(loc, "conn.add").into()
                }
            })
            .or_else(|| open_conn.map(|s| s.to_string())),
        "memory" => memory_selected.map(|s| s.to_string()),
        "channels" => channels_open.map(|key| match key {
            "feishu" => t(loc, "channels.feishu.title").into(),
            "weixin" => t(loc, "channels.weixin.title").into(),
            "sticks3" => t(loc, "channels.device.title").into(),
            other => other.to_string(),
        }),
        _ => None,
    }
}

fn secret_fields_json(fields: &[ConnSecretField]) -> Vec<serde_json::Value> {
    fields
        .iter()
        .filter(|field| !field.name.trim().is_empty())
        .map(|field| {
            let name = field.name.trim();
            let value = field.value.trim();
            if value.is_empty() {
                serde_json::json!({ "name": name })
            } else {
                serde_json::json!({ "name": name, "value": value })
            }
        })
        .collect()
}

fn secret_fields_from_entries(entries: &[McpSecretEntry]) -> Vec<ConnSecretField> {
    let mut fields: Vec<ConnSecretField> =
        entries.iter().map(ConnSecretField::from_entry).collect();
    if fields.is_empty() {
        fields.push(ConnSecretField::default());
    }
    fields
}

pub(crate) fn build_conn_json(f: &ConnForm, assign_id: bool) -> serde_json::Value {
    let id = f.id.clone().unwrap_or_else(|| {
        if assign_id {
            format!("conn-{}", (js_sys::Math::random() * 1e9) as u64)
        } else {
            "test".into()
        }
    });
    let transport = if f.kind == "http" {
        let auth = if f.auth == "oauth" { "oauth" } else { "none" };
        serde_json::json!({
            "kind": "http",
            "url": f.url.trim(),
            "headers": secret_fields_json(&f.headers),
            "auth": auth
        })
    } else {
        let args: Vec<String> = f.args.split_whitespace().map(|s| s.to_string()).collect();
        serde_json::json!({
            "kind": "stdio",
            "command": f.command.trim(),
            "args": args,
            "env": secret_fields_json(&f.env),
            "cwd": null
        })
    };
    serde_json::json!({ "id": id, "name": f.name.trim(), "enabled": f.enabled, "transport": transport })
}

pub(crate) fn conn_form_from_row(row: &ConnRow) -> ConnForm {
    match &row.transport {
        ConnTransport::Stdio {
            command, args, env, ..
        } => ConnForm {
            id: Some(row.id.clone()),
            name: row.name.clone(),
            kind: "stdio".into(),
            command: command.clone(),
            args: args.join(" "),
            url: String::new(),
            headers: vec![ConnSecretField::default()],
            env: secret_fields_from_entries(env),
            auth: "none".into(),
            enabled: row.enabled,
        },
        ConnTransport::Http { url, headers, auth } => ConnForm {
            id: Some(row.id.clone()),
            name: row.name.clone(),
            kind: "http".into(),
            command: String::new(),
            args: String::new(),
            url: url.clone(),
            headers: secret_fields_from_entries(headers),
            env: vec![ConnSecretField::default()],
            auth: if auth == "oauth" {
                "oauth".into()
            } else {
                "none".into()
            },
            enabled: row.enabled,
        },
    }
}

#[cfg(test)]
mod mcp_secret_form_tests {
    use super::{build_conn_json, conn_form_from_row};
    use crate::dto::{ConnForm, ConnRow, ConnSecretField, ConnTransport, McpSecretEntry};

    #[test]
    fn build_conn_json_omits_empty_secret_values() {
        let json = build_conn_json(
            &ConnForm {
                id: Some("conn-1".into()),
                name: "remote".into(),
                kind: "http".into(),
                url: "https://example.test/mcp".into(),
                headers: vec![ConnSecretField {
                    name: "Authorization".into(),
                    value: String::new(),
                    has_value: true,
                }],
                enabled: true,
                ..ConnForm::default()
            },
            false,
        );
        assert_eq!(
            json["transport"]["headers"],
            serde_json::json!([{ "name": "Authorization" }])
        );
        assert!(json["transport"]["headers"][0].get("value").is_none());
    }

    #[test]
    fn conn_form_from_row_never_copies_listed_values() {
        let row = ConnRow {
            id: "conn-1".into(),
            name: "remote".into(),
            enabled: true,
            transport: ConnTransport::Http {
                url: "https://example.test/mcp".into(),
                headers: vec![McpSecretEntry::plaintext("Authorization", "secret-value")],
                auth: "none".into(),
            },
        };
        let form = conn_form_from_row(&row);
        assert_eq!(form.headers[0].name, "Authorization");
        assert!(form.headers[0].value.is_empty());
        assert!(form.headers[0].has_value);
    }
}
