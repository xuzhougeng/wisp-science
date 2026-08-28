use super::*;

const SSH_RETRY_STOPPED_MARKER: &str = "ssh automatic retry stopped";

thread_local! {
    static RUN_REFRESH_INITIALIZED: Cell<bool> = const { Cell::new(false) };
    static SSH_RETRY_TOASTED_RUNS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

pub(crate) fn refresh_execution_contexts(into: RwSignal<Vec<ExecutionContext>>) {
    spawn_local(async move {
        let v = invoke("list_execution_contexts", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ExecutionContext>>(v) {
            // A no-op republish remounts the conversation runtime strip and
            // resizes the composer, which fires a chat scroll and dismisses
            // the selection popup (#1027).
            if into.with_untracked(|current| current != &list) {
                into.set(list);
            }
        }
    });
}

pub(crate) fn refresh_default_execution_context(into: RwSignal<Option<String>>) {
    spawn_local(async move {
        let v = invoke("get_default_execution_context", JsValue::UNDEFINED).await;
        if let Ok(id) = serde_wasm_bindgen::from_value::<Option<String>>(v) {
            into.set(id);
        }
    });
}

pub(crate) fn refresh_session_execution_contexts(
    into: RwSignal<HashSet<String>>,
    active_session: RwSignal<Option<String>>,
    session_id: String,
) {
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "sessionId": session_id.clone() })).unwrap();
        let Ok(value) = invoke_checked("list_session_execution_context_ids", args).await else {
            return;
        };
        let Ok(ids) = serde_wasm_bindgen::from_value::<Vec<String>>(value) else {
            return;
        };
        if active_session.get_untracked().as_deref() == Some(session_id.as_str()) {
            let next = ids.into_iter().collect::<HashSet<_>>();
            if into.with_untracked(|current| current != &next) {
                into.set(next);
            }
        }
    });
}

/// Render-relevant equality for the polled run list. `last_polled_at` is a
/// backend heartbeat that changes on every remote poll; treating it as a
/// change republishes every run card each poll cycle — the visible "page
/// refresh" that flickered cards and yanked the chat scroll back to the top.
/// `last_poll_error` stays in the comparison: the SSH-retry toast and the
/// card's error line depend on it.
fn run_lists_render_eq(current: &[RunSummary], next: &[RunSummary]) -> bool {
    current.len() == next.len()
        && current.iter().zip(next.iter()).all(|(a, b)| {
            let mut a = a.clone();
            let mut b = b.clone();
            a.last_polled_at = None;
            b.last_polled_at = None;
            a == b
        })
}

async fn fetch_runtimes(into: RwSignal<Vec<RuntimeInfo>>) -> Option<Vec<RuntimeInfo>> {
    let value = invoke("list_runtimes", JsValue::UNDEFINED).await;
    let list = serde_wasm_bindgen::from_value::<Vec<RuntimeInfo>>(value).ok()?;
    // This is polled every second while the agent runs. A no-op republish
    // rebuilds the memory environment panel, which recreates its filter input
    // mid-keystroke; only real changes may publish.
    if into.with_untracked(|current| current != &list) {
        into.set(list.clone());
    }
    Some(list)
}

pub(crate) fn refresh_runtimes(into: RwSignal<Vec<RuntimeInfo>>) {
    spawn_local(async move {
        let _ = fetch_runtimes(into).await;
    });
}

/// After an agent `python`/`r` cell finishes: pull the runtime list (the cell
/// may have lazily started or killed a process) and, when the open memory
/// environment shows that language, re-inspect it so its variable table
/// follows the agent without a manual sync click. The inspect is gated on the
/// *fresh* status — the signal the panel had could still say `missing` for a
/// runtime the cell just created, and inspecting a truly absent runtime would
/// replace the panel's content with a "not started" error.
pub(crate) fn refresh_runtime_environment_after_tool(
    language: String,
    runtime_environment: RwSignal<Option<RuntimeSlot>>,
    states: RwSignal<HashMap<String, RuntimeObjectState>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    locale: RwSignal<Locale>,
) {
    spawn_local(async move {
        let Some(list) = fetch_runtimes(runtimes).await else {
            return;
        };
        let Some(slot) = runtime_environment.get_untracked() else {
            return;
        };
        if slot.language != language {
            return;
        }
        let inspectable = list.iter().any(|info| {
            info.key.project_id == slot.project_id
                && info.key.context_id == slot.context_id
                && info.key.language == slot.language
                && matches!(info.status.as_str(), "ready" | "busy")
        });
        if inspectable {
            inspect_runtime_objects(
                runtime_binding_state_key(&slot.project_id, &slot.context_id, &slot.language),
                slot.project_id,
                slot.context_id,
                slot.language,
                locale,
                states,
                runtimes,
            );
        }
    });
}

pub(crate) fn refresh_runs(into: RwSignal<Vec<RunSummary>>, locale: RwSignal<Locale>) {
    spawn_local(async move {
        let v = invoke("list_runs", JsValue::UNDEFINED).await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<RunSummary>>(v) {
            let initialized = RUN_REFRESH_INITIALIZED.with(Cell::get);
            let stopped_runs = list
                .iter()
                .filter(|run| {
                    matches!(run.status.as_str(), "failed" | "lost")
                        && run.last_poll_error.as_deref().is_some_and(|error| {
                            error
                                .to_ascii_lowercase()
                                .contains(SSH_RETRY_STOPPED_MARKER)
                        })
                })
                .map(|run| run.id.clone())
                .collect::<Vec<_>>();
            let should_toast = SSH_RETRY_TOASTED_RUNS.with(|seen| {
                let mut seen = seen.borrow_mut();
                let mut added = false;
                for run_id in stopped_runs {
                    if seen.insert(run_id) {
                        added = true;
                    }
                }
                initialized && added
            });
            // The poll runs every second while the agent is busy. Setting the
            // signal unconditionally rebuilds every run card, which resets the
            // output panel's scroll to the top for a frame — a finished run
            // visibly jumped once per poll with nothing to show for it (#654).
            // The heartbeat field alone must not count as a change either, or
            // every remote poll keeps republishing the whole list.
            if into.with_untracked(|current| !run_lists_render_eq(current, &list)) {
                into.set(list);
                schedule_run_output_follow();
            }
            RUN_REFRESH_INITIALIZED.with(|ready| ready.set(true));
            if should_toast {
                show_warning_toast(&t(locale.get_untracked(), "runs.ssh_retry_stopped"));
            }
        }
    });
}

#[cfg(test)]
mod run_refresh_guard_tests {
    use super::run_lists_render_eq;
    use crate::dto::RunSummary;

    fn summary(last_polled_at: Option<i64>) -> RunSummary {
        RunSummary {
            id: "run-1".into(),
            frame_id: None,
            context_id: "local".into(),
            title: "job".into(),
            kind: "shell".into(),
            status: "running".into(),
            created_at: 1,
            started_at: Some(2),
            ended_at: None,
            exit_code: None,
            remote_workdir: None,
            timeout_secs: None,
            last_polled_at,
            last_poll_error: None,
            progress_json: "{}".into(),
            harvested_at: None,
            cleaned_at: None,
            cleanup_error: None,
            output_fingerprint: "fp".into(),
        }
    }

    #[test]
    fn heartbeat_only_changes_do_not_republish_the_run_list() {
        let current = vec![summary(Some(10))];
        assert!(run_lists_render_eq(&current, &[summary(Some(10))]));
        assert!(run_lists_render_eq(&current, &[summary(Some(20))]));
        assert!(run_lists_render_eq(&current, &[summary(None)]));

        let mut changed = summary(Some(10));
        changed.status = "succeeded".into();
        assert!(!run_lists_render_eq(&current, &[changed]));

        let mut changed = summary(Some(10));
        changed.last_poll_error = Some("ssh: connection lost".into());
        assert!(!run_lists_render_eq(&current, &[changed]));

        assert!(!run_lists_render_eq(&current, &[]));
        assert!(!run_lists_render_eq(
            &current,
            &[summary(Some(10)), summary(Some(11))]
        ));
    }
}

pub(crate) fn show_probe_stopped_toast(value: &JsValue, locale: RwSignal<Locale>) {
    let Ok(context) = serde_wasm_bindgen::from_value::<ExecutionContext>(value.clone()) else {
        return;
    };
    if context.last_probe_status.as_deref() == Some("error") {
        let key = if context
            .last_probe_error
            .as_deref()
            .is_some_and(|detail| classify_ssh_failure(detail) == SshFailKind::ProbeOutput)
        {
            "contexts.probe_incomplete"
        } else {
            "contexts.probe_stopped"
        };
        show_warning_toast(&t(locale.get_untracked(), key));
    }
}

pub(crate) fn context_capability_summary(ctx: &ExecutionContext) -> String {
    let parsed = serde_json::from_str::<serde_json::Value>(&ctx.capabilities_json).ok();
    let mut parts = Vec::new();
    if let Some(v) = parsed.as_ref() {
        let os = v.get("os").and_then(|x| x.as_str()).unwrap_or_default();
        let arch = v.get("arch").and_then(|x| x.as_str()).unwrap_or_default();
        match (os.is_empty(), arch.is_empty()) {
            (false, false) => parts.push(format!("{os}/{arch}")),
            (false, true) => parts.push(os.to_string()),
            (true, false) => parts.push(arch.to_string()),
            (true, true) => {}
        }
        for key in ["gpu_summary", "scheduler", "python", "r_version"] {
            if let Some(s) = v
                .get(key)
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
            {
                parts.push(s.to_string());
            }
        }
        if v.get("probe_skill").and_then(|x| x.as_str()).is_some()
            && v.get("gpu_summary").is_none_or(serde_json::Value::is_null)
        {
            parts.push("No GPU".into());
        }
        if let Some(privilege) = v.get("privilege").and_then(|x| x.as_str()) {
            parts.push(privilege.to_string());
        }
    }
    if parts.is_empty() {
        ctx.last_probe_status
            .clone()
            .unwrap_or_else(|| "not probed".into())
    } else {
        parts.join(" · ")
    }
}

pub(crate) fn language_display(language: &str) -> &str {
    match language {
        "r" => "R",
        "python" => "Python",
        other => other,
    }
}

/// Compute-context entries for the composer `@` menu: every execution context
/// as a server target, plus a runtime entry per available language on it.
/// Query tokens (split on non-alphanumerics, so `runtime_R` works) must all
/// match the entry's descriptive haystack.
fn context_display_label(ctx: &ExecutionContext) -> String {
    if ctx.label.trim().is_empty() {
        ctx.id.clone()
    } else {
        ctx.label.clone()
    }
}

/// Contexts a source file can bind its runtime to, as (id, label) pairs for the
/// preview picker. Same availability rule as the composer's `@` runtime entries,
/// so a file cannot bind to a runtime `@` would not offer. Empty means nothing
/// on this machine can run the language and there is no binding to make.
pub(crate) fn runtime_binding_options(
    contexts: &[ExecutionContext],
    language: &str,
) -> Vec<(String, String)> {
    contexts
        .iter()
        .filter(|ctx| context_runtime_available(ctx, language))
        .map(|ctx| (ctx.id.clone(), context_display_label(ctx)))
        .collect()
}

/// Resolve which context a script is actually bound to. A stored (or default)
/// binding that cannot host the language is not a binding — falling back to the
/// first context that can keeps the picker's displayed value and the context a
/// run is sent to from ever disagreeing.
pub(crate) fn resolve_runtime_binding(
    options: &[(String, String)],
    stored: Option<&str>,
) -> Option<String> {
    let hosted = |id: &str| options.iter().any(|(option, _)| option == id);
    stored
        .filter(|id| hosted(id))
        .map(str::to_string)
        .or_else(|| {
            hosted(LOCAL_CONTEXT_ID)
                .then(|| LOCAL_CONTEXT_ID.to_string())
                .or_else(|| options.first().map(|(id, _)| id.clone()))
        })
}

pub(crate) fn mention_compute_entries(
    query: &str,
    contexts: &[ExecutionContext],
) -> Vec<ComposerPickerItem> {
    let query = query.to_lowercase();
    let tokens: Vec<&str> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let matches = |haystack: String| tokens.iter().all(|t| haystack.to_lowercase().contains(t));
    let mut items = Vec::new();
    for ctx in contexts {
        let label = context_display_label(ctx);
        if matches(format!("server {} {} {label}", ctx.kind, ctx.id)) {
            items.push(ComposerPickerItem::Context {
                id: ctx.id.clone(),
                label: label.clone(),
            });
        }
        for language in ["python", "r"] {
            if context_runtime_available(ctx, language)
                && matches(format!(
                    "runtime {language} {} {} {label}",
                    ctx.kind, ctx.id
                ))
            {
                items.push(ComposerPickerItem::Runtime {
                    context_id: ctx.id.clone(),
                    context_label: label.clone(),
                    language: language.to_string(),
                });
            }
        }
    }
    items
}

fn context_runtime_available(ctx: &ExecutionContext, language: &str) -> bool {
    if ctx.kind == "local" && language == "python" {
        return true;
    }
    let config = serde_json::from_str::<serde_json::Value>(&ctx.config_json).unwrap_or_default();
    let capabilities =
        serde_json::from_str::<serde_json::Value>(&ctx.capabilities_json).unwrap_or_default();
    let has_value = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty())
    };
    match language {
        "python" => {
            ["python_executable", "python_path"]
                .iter()
                .any(|key| has_value(&config, key))
                || has_value(&capabilities, "python_executable")
        }
        "r" => {
            if ["rscript_executable", "rscript_path"]
                .iter()
                .any(|key| has_value(&config, key))
            {
                return true;
            }
            if has_value(&capabilities, "rscript_executable") {
                return capabilities
                    .get("r_jsonlite")
                    .and_then(|value| value.as_bool())
                    != Some(false);
            }
            ctx.kind == "local" && ctx.last_probe_status.as_deref() != Some("ok")
        }
        _ => false,
    }
}

#[cfg(test)]
mod runtime_slot_tests {
    use super::{
        classify_ssh_failure, context_runtime_available, is_ssh_setup_error,
        mention_compute_entries, runtime_object_matches, session_runtime_groups,
        session_runtime_strip_view, ssh_connectivity_gap, ssh_fail_cause_keys,
        ssh_setup_context_id, ComposerPickerItem, RuntimeSlot, SshFailKind,
    };
    use crate::dto::{ExecutionContext, RuntimeObject};
    use crate::i18n::Locale;
    use std::collections::HashSet;

    fn context(
        kind: &str,
        capabilities_json: &str,
        probe_status: Option<&str>,
    ) -> ExecutionContext {
        ExecutionContext {
            id: if kind == "local" { "local" } else { "ssh:test" }.into(),
            kind: kind.into(),
            label: "Test".into(),
            config_json: "{}".into(),
            capabilities_json: capabilities_json.into(),
            last_probe_status: probe_status.map(str::to_string),
            last_probe_error: None,
        }
    }

    #[test]
    fn ssh_connectivity_gap_requires_successful_probe() {
        assert!(ssh_connectivity_gap(&context("local", "{}", None)).is_none());
        assert_eq!(
            ssh_connectivity_gap(&context("ssh", "{}", None)).as_deref(),
            Some("not probed yet")
        );
        assert_eq!(
            ssh_connectivity_gap(&context("ssh", "{}", Some("error"))).as_deref(),
            Some("probe failed")
        );
        assert!(ssh_connectivity_gap(&context("ssh", "{}", Some("ok"))).is_none());
    }

    #[test]
    fn ssh_setup_error_helpers_detect_and_parse_context() {
        let err = "SSH connectivity is not confirmed for `ssh:insertsbio_public`: no successful probe yet.";
        assert!(is_ssh_setup_error(err));
        assert_eq!(
            ssh_setup_context_id(None, err).as_deref(),
            Some("ssh:insertsbio_public")
        );
        assert_eq!(
            ssh_setup_context_id(Some("ssh:other"), err).as_deref(),
            Some("ssh:other")
        );
        assert!(!is_ssh_setup_error("Remote directory empty"));
    }

    #[test]
    fn classify_ssh_failure_maps_permission_denied_to_auth() {
        let detail =
            "SSH probe failed with exit 255: user@host: Permission denied (publickey,password).";
        assert_eq!(classify_ssh_failure(detail), SshFailKind::Auth);
        assert!(ssh_fail_cause_keys(SshFailKind::Auth).len() >= 3);
        assert_eq!(
            classify_ssh_failure("SSH password authentication failed for `gpu-box`"),
            SshFailKind::PasswordAuth
        );
        assert_eq!(
            classify_ssh_failure("SSH key authentication failed for `gpu-box`"),
            SshFailKind::KeyAuth
        );
        assert_eq!(
            classify_ssh_failure("Connection timed out"),
            SshFailKind::Timeout
        );
        assert_eq!(
            classify_ssh_failure("identity file is not accessible"),
            SshFailKind::IdentityMissing
        );
        assert_eq!(
            classify_ssh_failure(
                "SSH connection succeeded, but the environment probe could not read operating system information"
            ),
            SshFailKind::ProbeOutput
        );
    }

    #[test]
    fn optional_r_slot_distinguishes_unknown_available_and_missing() {
        assert!(context_runtime_available(
            &context("local", "{}", None),
            "r"
        ));
        assert!(!context_runtime_available(
            &context("local", r#"{"rscript_executable":null}"#, Some("ok")),
            "r"
        ));
        assert!(context_runtime_available(
            &context(
                "ssh",
                r#"{"rscript_executable":"/usr/bin/Rscript","r_jsonlite":true}"#,
                Some("ok")
            ),
            "r"
        ));
        assert!(!context_runtime_available(
            &context(
                "ssh",
                r#"{"rscript_executable":"/usr/bin/Rscript","r_jsonlite":false}"#,
                Some("ok")
            ),
            "r"
        ));
    }

    #[test]
    fn binding_options_offer_only_contexts_that_can_host_the_language() {
        let contexts = vec![
            context("local", "{}", None),
            context(
                "ssh",
                r#"{"rscript_executable":"/usr/bin/Rscript","r_jsonlite":false}"#,
                Some("ok"),
            ),
        ];
        // Local always hosts Python; the SSH host has R but no jsonlite, so it
        // cannot host an R runtime and must not be offered as a binding.
        let r = super::runtime_binding_options(&contexts, "r");
        assert_eq!(r, vec![("local".to_string(), "Test".to_string())]);
        let python = super::runtime_binding_options(&contexts, "python");
        assert_eq!(
            python.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            vec!["local"]
        );
    }

    #[test]
    fn binding_resolves_to_a_context_that_can_actually_host_the_language() {
        let options = vec![
            ("local".to_string(), "Local".to_string()),
            ("ssh:gpu".to_string(), "GPU".to_string()),
        ];
        // A stored binding is honoured...
        assert_eq!(
            super::resolve_runtime_binding(&options, Some("ssh:gpu")),
            Some("ssh:gpu".to_string())
        );
        // ...unless that context cannot host the language, in which case the
        // picker would show one context while runs went to another.
        assert_eq!(
            super::resolve_runtime_binding(&options, Some("ssh:gone")),
            Some("local".to_string())
        );
        assert_eq!(
            super::resolve_runtime_binding(&options, None),
            Some("local".to_string())
        );
        // Local cannot host R without Rscript; fall back to one that can.
        let r_only = vec![("ssh:gpu".to_string(), "GPU".to_string())];
        assert_eq!(
            super::resolve_runtime_binding(&r_only, None),
            Some("ssh:gpu".to_string())
        );
        // Nothing can host it: no binding, so no run controls.
        assert_eq!(super::resolve_runtime_binding(&[], Some("local")), None);
    }

    #[test]
    fn console_echo_prefixes_every_submitted_line() {
        assert_eq!(
            super::console_echo("library(Seurat)\nlibrary(dplyr)", Locale::En),
            "> library(Seurat)\n> library(dplyr)"
        );
    }

    #[test]
    fn console_echo_bounds_long_script_previews() {
        let code = (1..=40)
            .map(|line| format!("line_{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let echo = super::console_echo(&code, Locale::En);
        assert_eq!(echo.lines().count(), 11);
        assert!(echo.contains("> line_1"));
        assert!(echo.contains("> … 30 submitted lines omitted …"));
        assert!(echo.contains("> line_40"));
        assert!(!echo.contains("> line_20"));

        let zh_echo = super::console_echo(&code, Locale::Zh);
        assert!(zh_echo.contains("> … 已省略 30 行提交代码 …"));
    }

    #[test]
    fn closed_worker_error_is_user_facing() {
        assert_eq!(
            crate::i18n::localize_backend(Locale::Zh, "kernel worker closed protocol stdout"),
            "Runtime 进程意外退出，请重启后再运行代码。"
        );
    }

    #[test]
    fn mention_entries_match_servers_and_runtimes() {
        let contexts = vec![context(
            "ssh",
            r#"{"rscript_executable":"/usr/bin/Rscript","r_jsonlite":true}"#,
            Some("ok"),
        )];
        // Empty query lists the server plus its available runtime.
        let all = mention_compute_entries("", &contexts);
        assert!(all.iter().any(
            |item| matches!(item, ComposerPickerItem::Context { id, .. } if id == "ssh:test")
        ));
        assert!(all.iter().any(|item| matches!(
            item,
            ComposerPickerItem::Runtime { language, .. } if language == "r"
        )));
        // `runtime_R` style queries tokenize on the underscore and drop the
        // server entry (its haystack has no "runtime" token).
        let runtimes = mention_compute_entries("runtime_R", &contexts);
        assert!(runtimes.iter().any(|item| matches!(
            item,
            ComposerPickerItem::Runtime { language, .. } if language == "r"
        )));
        assert!(!runtimes
            .iter()
            .any(|item| matches!(item, ComposerPickerItem::Context { .. })));
        // Server label match.
        assert!(mention_compute_entries("test", &contexts)
            .iter()
            .any(|item| matches!(item, ComposerPickerItem::Context { .. })));
        assert!(mention_compute_entries("nomatch", &contexts).is_empty());
    }

    fn slot(context_id: &str, language: &str) -> RuntimeSlot {
        RuntimeSlot {
            project_id: "p".into(),
            project_label: "p".into(),
            context_id: context_id.into(),
            context_label: context_id.into(),
            language: language.into(),
            available: true,
            info: None,
        }
    }

    #[test]
    fn session_runtime_groups_keep_local_and_attached_remotes() {
        let contexts = vec![
            context("local", "{}", None),
            context(
                "ssh",
                r#"{"rscript_executable":"/usr/bin/Rscript","r_jsonlite":true}"#,
                Some("ok"),
            ),
        ];
        let slots = vec![
            slot("local", "python"),
            slot("local", "r"),
            slot("ssh:test", "python"),
            slot("ssh:test", "r"),
        ];
        let attached = HashSet::new();
        let local_only = session_runtime_groups(slots.clone(), &contexts, &attached);
        assert_eq!(local_only.len(), 1);
        assert_eq!(local_only[0].context_id, "local");
        assert_eq!(local_only[0].slots.len(), 2);

        let mut attached = HashSet::new();
        attached.insert("ssh:test".into());
        let both = session_runtime_groups(slots, &contexts, &attached);
        assert_eq!(
            both.iter()
                .map(|group| group.context_id.as_str())
                .collect::<Vec<_>>(),
            vec!["local", "ssh:test"]
        );
    }

    #[test]
    fn session_runtime_strip_view_ignores_unattached_context_churn() {
        let local_only = vec![context("local", "{}", None)];
        let with_wsl = vec![
            context("local", "{}", None),
            ExecutionContext {
                id: "wsl:Ubuntu-24.04".into(),
                kind: "wsl".into(),
                label: "Ubuntu-24.04".into(),
                config_json: r#"{"distro":"Ubuntu-24.04"}"#.into(),
                capabilities_json: "{}".into(),
                last_probe_status: None,
                last_probe_error: None,
            },
        ];
        let slots = vec![slot("local", "python"), slot("local", "r")];
        let attached = HashSet::new();
        assert_eq!(
            session_runtime_strip_view(slots.clone(), &local_only, &attached),
            session_runtime_strip_view(slots, &with_wsl, &attached)
        );
    }

    #[test]
    fn runtime_object_filter_matches_name_type_or_summary() {
        let object = RuntimeObject {
            name: "sce".into(),
            type_name: "Seurat".into(),
            summary: "17775 × 70634".into(),
            size_bytes: None,
        };
        assert!(runtime_object_matches(&object, ""));
        assert!(runtime_object_matches(&object, "SCE"));
        assert!(runtime_object_matches(&object, "seurat"));
        assert!(runtime_object_matches(&object, "17775"));
        assert!(!runtime_object_matches(&object, "adata"));
    }
}

pub(crate) fn runtime_slots(
    runtimes: Vec<RuntimeInfo>,
    contexts: &[ExecutionContext],
    active_project: Option<ProjectInfo>,
    projects: &[ProjectSummary],
) -> Vec<RuntimeSlot> {
    let project_label = |id: &str| {
        active_project
            .as_ref()
            .filter(|project| project.id == id)
            .map(|project| project.name.clone())
            .or_else(|| {
                projects
                    .iter()
                    .find(|project| project.id == id)
                    .map(|project| project.name.clone())
            })
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| id.to_string())
    };
    let context_label = |id: &str| {
        contexts
            .iter()
            .find(|context| context.id == id)
            .map(|context| {
                if context.label.trim().is_empty() {
                    context.id.clone()
                } else {
                    context.label.clone()
                }
            })
            .unwrap_or_else(|| id.to_string())
    };

    let mut present = HashSet::new();
    let mut slots = runtimes
        .into_iter()
        .map(|info| {
            present.insert((
                info.key.project_id.clone(),
                info.key.context_id.clone(),
                info.key.language.clone(),
            ));
            RuntimeSlot {
                project_id: info.key.project_id.clone(),
                project_label: project_label(&info.key.project_id),
                context_id: info.key.context_id.clone(),
                context_label: context_label(&info.key.context_id),
                language: info.key.language.clone(),
                available: true,
                info: Some(info),
            }
        })
        .collect::<Vec<_>>();

    if let Some(project) = active_project.as_ref() {
        for context in contexts {
            for language in ["python", "r"] {
                let key = (project.id.clone(), context.id.clone(), language.to_string());
                if present.insert(key) {
                    slots.push(RuntimeSlot {
                        project_id: project.id.clone(),
                        project_label: project_label(&project.id),
                        context_id: context.id.clone(),
                        context_label: context_label(&context.id),
                        language: language.to_string(),
                        available: context_runtime_available(context, language),
                        info: None,
                    });
                }
            }
        }
    }
    slots.sort_by(|left, right| {
        left.project_id
            .cmp(&right.project_id)
            .then_with(|| left.context_id.cmp(&right.context_id))
            .then_with(|| left.language.cmp(&right.language))
    });
    slots
}

/// Local is always on the conversation; remotes appear only once attached.
pub(crate) fn session_visible_contexts<'a>(
    contexts: &'a [ExecutionContext],
    attached: &HashSet<String>,
) -> Vec<&'a ExecutionContext> {
    contexts
        .iter()
        .filter(|context| context.kind == "local" || attached.contains(&context.id))
        .collect()
}

#[derive(Clone)]
pub(crate) struct SessionRuntimeGroup {
    pub context_id: String,
    pub context_label: String,
    pub kind: String,
    pub slots: Vec<RuntimeSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionRuntimeStripChip {
    pub context_id: String,
    pub context_label: String,
    pub language: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionRuntimeStripGroup {
    pub context_id: String,
    pub context_label: String,
    pub kind: String,
    pub chips: Vec<SessionRuntimeStripChip>,
}

pub(crate) fn session_runtime_strip_view(
    slots: Vec<RuntimeSlot>,
    contexts: &[ExecutionContext],
    attached: &HashSet<String>,
) -> Vec<SessionRuntimeStripGroup> {
    session_runtime_groups(slots, contexts, attached)
        .into_iter()
        .map(|group| SessionRuntimeStripGroup {
            context_id: group.context_id,
            context_label: group.context_label,
            kind: group.kind,
            chips: group
                .slots
                .into_iter()
                .map(|slot| {
                    let status = slot_status(&slot);
                    SessionRuntimeStripChip {
                        context_id: slot.context_id,
                        context_label: slot.context_label,
                        language: slot.language,
                        status,
                    }
                })
                .collect(),
        })
        .collect()
}

pub(crate) fn session_runtime_groups(
    slots: Vec<RuntimeSlot>,
    contexts: &[ExecutionContext],
    attached: &HashSet<String>,
) -> Vec<SessionRuntimeGroup> {
    session_visible_contexts(contexts, attached)
        .into_iter()
        .filter_map(|context| {
            let context_slots = slots
                .iter()
                .filter(|slot| slot.context_id == context.id)
                .cloned()
                .collect::<Vec<_>>();
            if context_slots.is_empty() {
                return None;
            }
            Some(SessionRuntimeGroup {
                context_id: context.id.clone(),
                context_label: if context.label.trim().is_empty() {
                    context.id.clone()
                } else {
                    context.label.clone()
                },
                kind: context.kind.clone(),
                slots: context_slots,
            })
        })
        .collect()
}

pub(crate) fn slot_status(slot: &RuntimeSlot) -> String {
    slot.info
        .as_ref()
        .map(|info| info.status.clone())
        .unwrap_or_else(|| {
            if slot.available {
                "missing".into()
            } else {
                "unavailable".into()
            }
        })
}

pub(crate) fn runtime_object_matches(object: &RuntimeObject, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    object.name.to_ascii_lowercase().contains(&query)
        || object.type_name.to_ascii_lowercase().contains(&query)
        || object.summary.to_ascii_lowercase().contains(&query)
}

pub(crate) fn open_runtime_environment(
    slot: RuntimeSlot,
    runtime_environment: RwSignal<Option<RuntimeSlot>>,
    object_states: RwSignal<HashMap<String, RuntimeObjectState>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    locale: RwSignal<Locale>,
) {
    let can_inspect = slot
        .info
        .as_ref()
        .is_some_and(|info| matches!(info.status.as_str(), "ready" | "busy"));
    // Key object snapshots by the stable binding, not the process runtime id:
    // the panel then keeps showing data across lazy starts and restarts, and
    // agent-driven refreshes land where the panel reads.
    let inspect_key = runtime_binding_state_key(&slot.project_id, &slot.context_id, &slot.language);
    let inspect_project = slot.project_id.clone();
    let inspect_context = slot.context_id.clone();
    let inspect_language = slot.language.clone();
    runtime_environment.set(Some(slot));
    if can_inspect {
        inspect_runtime_objects(
            inspect_key,
            inspect_project,
            inspect_context,
            inspect_language,
            locale,
            object_states,
            runtimes,
        );
    }
}

fn invoke_runtime_control(
    command: &'static str,
    args: serde_json::Value,
    locale: RwSignal<Locale>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
) {
    spawn_local(async move {
        let args = to_value(&args).unwrap();
        match invoke_checked(command, args).await {
            Ok(_) => refresh_runtimes(runtimes),
            Err(error) => {
                let message = localize_backend(locale.get_untracked(), &js_error_text(error));
                show_toast(&message);
                refresh_runtimes(runtimes);
            }
        }
    });
}

pub(crate) fn runtime_status_label(locale: Locale, status: &str) -> String {
    let key = match status {
        "starting" => "runtime.starting",
        "ready" => "runtime.ready",
        "busy" => "runtime.busy",
        "stopping" => "runtime.stopping",
        "dead" => "runtime.dead",
        "unavailable" => "runtime.unavailable",
        _ => "runtime.missing",
    };
    t(locale, key).into()
}

pub(crate) fn inspect_runtime_objects(
    state_key: String,
    project_id: String,
    context_id: String,
    language: String,
    locale: RwSignal<Locale>,
    states: RwSignal<HashMap<String, RuntimeObjectState>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
) {
    states.update(|states| {
        let state = states.entry(state_key.clone()).or_default();
        state.loading = true;
        state.error = None;
    });
    spawn_local(async move {
        let args = to_value(&serde_json::json!({
            "projectId": project_id,
            "contextId": context_id,
            "language": language,
        }))
        .unwrap();
        let result = match invoke_checked("inspect_runtime", args).await {
            Ok(value) => serde_wasm_bindgen::from_value::<RuntimeObjectList>(value)
                .map_err(|error| error.to_string()),
            Err(error) => Err(localize_backend(
                locale.get_untracked(),
                &js_error_text(error),
            )),
        };
        states.update(|states| {
            let state = states.entry(state_key).or_default();
            state.loading = false;
            match result {
                Ok(snapshot) => {
                    state.snapshot = Some(snapshot);
                    state.error = None;
                }
                Err(error) => state.error = Some(error),
            }
        });
        refresh_runtimes(runtimes);
    });
}

/// Mirrors `wisp_runtime::LOCAL_CONTEXT_ID`. `ui/` is a separate workspace and
/// cannot depend on the runtime crate, so the default binding is spelled here.
pub(crate) const LOCAL_CONTEXT_ID: &str = "local";

/// Stable object-inspection key for the runtime bound to a center source file.
/// Unlike the process runtime id, this survives the lazy first start and lets
/// the inspector publish variables immediately after selected code runs.
pub(crate) fn runtime_binding_state_key(
    project_id: &str,
    context_id: &str,
    language: &str,
) -> String {
    format!("binding:{project_id}:{context_id}:{language}")
}

/// Console log per previewed file path. Ephemeral like the runtime it mirrors:
/// a log that outlived its process would describe variables that no longer
/// exist. Use "add to chat" to hand a result to the agent.
pub(crate) type RuntimeConsoles = HashMap<String, String>;

/// Plot history per previewed file path: base64 PNG snapshots, oldest first.
/// Ephemeral like the console it accompanies.
pub(crate) type RuntimePlots = HashMap<String, Vec<String>>;

/// R and Python consoles echo submitted code behind a prompt. Keeping that here
/// is what lets one flat log stay readable as alternating input and output.
fn console_echo(code: &str, locale: Locale) -> String {
    const MAX_LINES: usize = 12;
    const HEAD_LINES: usize = 7;
    const TAIL_LINES: usize = 3;

    let lines = code.lines().collect::<Vec<_>>();
    let visible = if lines.len() <= MAX_LINES {
        lines.into_iter().map(str::to_string).collect::<Vec<_>>()
    } else {
        let omitted = lines.len() - HEAD_LINES - TAIL_LINES;
        lines[..HEAD_LINES]
            .iter()
            .map(|line| (*line).to_string())
            .chain(std::iter::once(tf(
                locale,
                "runtime.console_omitted",
                &[("n", &omitted.to_string())],
            )))
            .chain(
                lines[lines.len() - TAIL_LINES..]
                    .iter()
                    .map(|line| (*line).to_string()),
            )
            .collect::<Vec<_>>()
    };
    visible
        .into_iter()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_console(consoles: RwSignal<RuntimeConsoles>, path: &str, text: &str) {
    consoles.update(|logs| {
        let log = logs.entry(path.to_string()).or_default();
        if !log.is_empty() {
            log.push('\n');
        }
        log.push_str(text);
    });
}

/// The signals a file preview needs to run code against its bound runtime. They
/// always travel together; passing them as one keeps the run helpers readable.
#[derive(Clone, Copy)]
pub(crate) struct RuntimeRunCtx {
    pub(crate) consoles: RwSignal<RuntimeConsoles>,
    pub(crate) plots: RwSignal<RuntimePlots>,
    pub(crate) busy: RwSignal<Option<String>>,
    pub(crate) runtimes: RwSignal<Vec<RuntimeInfo>>,
    pub(crate) project: RwSignal<Option<ProjectInfo>>,
    pub(crate) object_states: RwSignal<HashMap<String, RuntimeObjectState>>,
    pub(crate) inspector_open: RwSignal<bool>,
    pub(crate) locale: RwSignal<Locale>,
}

/// Run `code` from the `path` preview against its bound runtime and append the
/// result to that file's console. The runtime starts lazily, so a not-yet-running
/// binding needs no separate Start click.
pub(crate) fn run_in_runtime(
    path: String,
    context_id: String,
    language: String,
    code: String,
    locale: Locale,
    ctx: RuntimeRunCtx,
) {
    let code = code.trim().to_string();
    // ponytail: one run at a time across all files. Key `busy` by path if two
    // runtimes on different contexts ever need to run concurrently.
    if code.is_empty() || ctx.busy.get_untracked().is_some() {
        return;
    }
    // Only set when actually closed: a redundant `set(true)` still notifies
    // subscribers, remounting the console/plots panes mid-session and wiping
    // the console's local input history.
    if !ctx.inspector_open.get_untracked() {
        ctx.inspector_open.set(true);
    }
    append_console(ctx.consoles, &path, &console_echo(&code, locale));
    ctx.busy.set(Some(path.clone()));
    spawn_local(async move {
        let args = to_value(&serde_json::json!({
            "contextId": context_id,
            "language": language,
            "code": code,
        }))
        .unwrap();
        let output = match invoke_checked("execute_runtime", args).await {
            Ok(value) => match serde_wasm_bindgen::from_value::<RuntimeExecutionSummary>(value) {
                Ok(summary) => summary,
                Err(error) => RuntimeExecutionSummary {
                    text: error.to_string(),
                    plots: Vec::new(),
                },
            },
            Err(error) => RuntimeExecutionSummary {
                text: localize_backend(locale, &js_error_text(error)),
                plots: Vec::new(),
            },
        };
        append_console(ctx.consoles, &path, &output.text);
        if !output.plots.is_empty() {
            ctx.plots.update(|plots| {
                plots.entry(path.clone()).or_default().extend(output.plots);
            });
        }
        ctx.busy.set(None);
        // Execution may have lazily created the process. Inspect by its stable
        // binding key so variables appear without waiting for list_runtimes to
        // reveal a new process id first.
        if let Some(project) = ctx.project.get_untracked() {
            inspect_runtime_objects(
                runtime_binding_state_key(&project.id, &context_id, &language),
                project.id,
                context_id,
                language,
                ctx.locale,
                ctx.object_states,
                ctx.runtimes,
            );
        } else {
            refresh_runtimes(ctx.runtimes);
        }
    });
}

/// One-line quote the selection popup actions (add to chat / explain) send for
/// a runtime variable. `summary`/`size` arrive display-normalized ("—" = none).
pub(crate) fn runtime_object_quote(
    language: &str,
    name: &str,
    type_name: &str,
    summary: &str,
    size: &str,
) -> String {
    let mut quote = format!("[{language} runtime] {name}: {type_name}");
    if summary != "—" {
        quote.push_str(" = ");
        quote.push_str(summary);
    }
    if size != "—" {
        quote.push_str(&format!(" ({size})"));
    }
    quote
}

fn runtime_environment_viewport() -> (i32, i32) {
    web_sys::window()
        .map(|window| {
            let width = window
                .inner_width()
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or(1280.0) as i32;
            let height = window
                .inner_height()
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or(720.0) as i32;
            (width, height)
        })
        .unwrap_or((1280, 720))
}

fn clamp_runtime_environment_position(x: i32, y: i32) -> (i32, i32) {
    const MARGIN: i32 = 16;
    const PANEL_WIDTH: i32 = 620;
    const PANEL_HEIGHT: i32 = 560;
    let (viewport_width, viewport_height) = runtime_environment_viewport();
    let width = PANEL_WIDTH.min((viewport_width - MARGIN * 2).max(0));
    let height = PANEL_HEIGHT.min((viewport_height - MARGIN * 2).max(0));
    (
        x.clamp(MARGIN, (viewport_width - width - MARGIN).max(MARGIN)),
        y.clamp(MARGIN, (viewport_height - height - MARGIN).max(MARGIN)),
    )
}

fn default_runtime_environment_position() -> (i32, i32) {
    let (viewport_width, viewport_height) = runtime_environment_viewport();
    clamp_runtime_environment_position(viewport_width - 636, (viewport_height - 560) / 2)
}

#[component]
pub(crate) fn RuntimeEnvironmentPanel(
    selected: RwSignal<Option<RuntimeSlot>>,
    pinned: RwSignal<bool>,
    position: RwSignal<(i32, i32)>,
    context_modal: RwSignal<Option<(String, ContextModalKind)>>,
    locale: RwSignal<Locale>,
    states: RwSignal<HashMap<String, RuntimeObjectState>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    contexts: RwSignal<Vec<ExecutionContext>>,
    active_project: RwSignal<Option<ProjectInfo>>,
    projects: RwSignal<Vec<ProjectSummary>>,
    selection_popup: RwSignal<Option<(String, Option<String>, i32, i32)>>,
) -> impl IntoView {
    let drag_start = Rc::new(Cell::new(None::<(i32, i32, i32, i32, i32)>));
    let dragging = create_rw_signal(false);
    let filter_query = create_rw_signal(String::new());
    create_effect(move |_| {
        let _ = selected.get();
        filter_query.set(String::new());
    });

    move || {
        selected.get().map(|mut slot| {
        let drag_start_down = drag_start.clone();
        let drag_start_move = drag_start.clone();
        let drag_start_up = drag_start.clone();
        let drag_start_cancel = drag_start.clone();
        slot.info = runtimes.get().into_iter().find(|runtime| {
            runtime.key.project_id == slot.project_id
                && runtime.key.context_id == slot.context_id
                && runtime.key.language == slot.language
        });
        let language_label = if slot.language == "r" { "R" } else { "Python" };
        let status = slot_status(&slot);
        let status_class = format!("runtime-status {status}");
        // The binding key survives the process: a runtime the agent lazily
        // started (or restarted) mid-conversation publishes its inspection
        // under the same key this panel reads.
        let state_key = runtime_binding_state_key(&slot.project_id, &slot.context_id, &slot.language);
        let has_runtime = slot.info.is_some();
        let can_refresh = status == "ready";
        let refresh_state_key = state_key.clone();
        let loading_state_key = state_key.clone();
        let content_state_key = state_key;
        let refresh_project = slot.project_id.clone();
        let refresh_context = slot.context_id.clone();
        let refresh_language = slot.language.clone();
        let sibling_slots = runtime_slots(
            runtimes.get(),
            &contexts.get(),
            active_project.get(),
            &projects.get(),
        )
        .into_iter()
        .filter(|sibling| {
            sibling.project_id == slot.project_id && sibling.context_id == slot.context_id
        })
        .collect::<Vec<_>>();
        let selected_language = slot.language.clone();

        view! {
            <section class="runtime-environment-panel" role="region"
                class:is-pinned=move || pinned.get()
                class:is-dragging=move || dragging.get()
                style=move || {
                    let (x, y) = position.get();
                    format!("--runtime-environment-x:{x}px;--runtime-environment-y:{y}px")
                }
                aria-label=tf(locale.get(), "runtime.environment_title", &[("language", language_label)])>
                <div class="runtime-environment-head">
                    <div class="runtime-environment-title"
                        on:pointerdown=move |event: web_sys::PointerEvent| {
                            if !pinned.get_untracked() || event.button() != 0 {
                                return;
                            }
                            event.prevent_default();
                            let Some(target) = event.target()
                                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                            else {
                                return;
                            };
                            let _ = target.set_pointer_capture(event.pointer_id());
                            let (x, y) = position.get_untracked();
                            drag_start_down.set(Some((
                                event.client_x(),
                                event.client_y(),
                                x,
                                y,
                                event.pointer_id(),
                            )));
                            dragging.set(true);
                        }
                        on:pointermove=move |event: web_sys::PointerEvent| {
                            let Some((start_x, start_y, origin_x, origin_y, _)) = drag_start_move.get() else {
                                return;
                            };
                            event.prevent_default();
                            position.set(clamp_runtime_environment_position(
                                origin_x + event.client_x() - start_x,
                                origin_y + event.client_y() - start_y,
                            ));
                        }
                        on:pointerup=move |event: web_sys::PointerEvent| {
                            if let Some((_, _, _, _, pointer_id)) = drag_start_up.take() {
                                if let Some(target) = event.target()
                                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                                {
                                    let _ = target.release_pointer_capture(pointer_id);
                                }
                            }
                            dragging.set(false);
                        }
                        on:pointercancel=move |_| {
                            drag_start_cancel.set(None);
                            dragging.set(false);
                        }>
                        <h3>{tf(locale.get(), "runtime.environment_title", &[("language", language_label)])}</h3>
                        <span>{format!("{} · {}", slot.project_label, slot.context_label)}</span>
                    </div>
                    {(sibling_slots.len() > 1).then(|| {
                        let tabs = sibling_slots.clone();
                        view! {
                            <div class="runtime-environment-langs" role="tablist"
                                aria-label=t(locale.get(), "runtime.switch_language_list")>
                                {tabs.into_iter().map(|sibling| {
                                    let language = sibling.language.clone();
                                    let click_slot = sibling.clone();
                                    let current_language = selected_language.clone();
                                    let active = language == selected_language;
                                    let label = language_display(&language).to_string();
                                    view! {
                                        <button type="button" role="tab"
                                            class="runtime-environment-lang"
                                            class:active=active
                                            data-testid="runtime-environment-lang"
                                            data-runtime-language=language.clone()
                                            aria-selected=active.to_string()
                                            title=tf(locale.get(), "runtime.switch_language", &[("language", &label)])
                                            aria-label=tf(locale.get(), "runtime.switch_language", &[("language", &label)])
                                            on:click=move |_| {
                                                if language == current_language {
                                                    return;
                                                }
                                                open_runtime_environment(
                                                    click_slot.clone(),
                                                    selected,
                                                    states,
                                                    runtimes,
                                                    locale,
                                                );
                                            }>
                                            {label}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>
                        }
                    })}
                    <button type="button" class="runtime-environment-pin"
                        class:active=move || pinned.get()
                        aria-pressed=move || pinned.get().to_string()
                        title=move || if pinned.get() {
                            t(locale.get(), "runtime.unpin_environment")
                        } else {
                            t(locale.get(), "runtime.pin_environment")
                        }
                        aria-label=move || if pinned.get() {
                            t(locale.get(), "runtime.unpin_environment")
                        } else {
                            t(locale.get(), "runtime.pin_environment")
                        }
                        on:click=move |_| {
                            if pinned.get_untracked() {
                                pinned.set(false);
                                if context_modal.get_untracked().is_none() {
                                    selected.set(None);
                                }
                            } else {
                                position.set(default_runtime_environment_position());
                                pinned.set(true);
                                context_modal.set(None);
                            }
                        }>{compose_icon("pin")}</button>
                    <span class=status_class>{runtime_status_label(locale.get(), &status)}</span>
                    <button type="button" class="runtime-environment-refresh"
                        title=t(locale.get(), "runtime.inspect_objects")
                        aria-label=t(locale.get(), "runtime.inspect_objects")
                        disabled=move || !can_refresh || states.with(|states| {
                            states.get(&loading_state_key).is_some_and(|state| state.loading)
                        })
                        on:click=move |_| inspect_runtime_objects(
                            refresh_state_key.clone(),
                            refresh_project.clone(),
                            refresh_context.clone(),
                            refresh_language.clone(),
                            locale,
                            states,
                            runtimes,
                        )>{compose_icon("sync")}</button>
                    <button type="button" class="runtime-environment-close"
                        title=t(locale.get(), "runtime.close_environment")
                        aria-label=t(locale.get(), "runtime.close_environment")
                        on:click=move |_| {
                            selected.set(None);
                            pinned.set(false);
                            dragging.set(false);
                        }>{compose_icon("close")}</button>
                </div>
                <div class="runtime-environment-filter">
                    <input type="search" data-testid="runtime-object-filter"
                        autocomplete="off"
                        aria-label=t(locale.get(), "runtime.objects_filter")
                        placeholder=t(locale.get(), "runtime.objects_filter_ph")
                        prop:value=move || filter_query.get()
                        on:input=move |ev| filter_query.set(event_target_value(&ev)) />
                </div>
                <div class="runtime-environment-table-head" aria-hidden="true">
                    <span>{t(locale.get(), "runtime.object_name")}</span>
                    <span>{t(locale.get(), "runtime.object_type")}</span>
                    <span>{t(locale.get(), "runtime.object_value")}</span>
                    <span>{t(locale.get(), "runtime.object_size")}</span>
                </div>
                <div class="runtime-environment-body">
                    {move || {
                        let state = states.with(|states| {
                            states.get(&content_state_key).cloned().unwrap_or_default()
                        });
                        // No process and nothing inspected yet: a last-known
                        // snapshot (from before a stop or crash) stays visible
                        // next to the status chip instead of vanishing.
                        if !has_runtime && state.snapshot.is_none() && state.error.is_none() {
                            return view! {
                                <div class="runtime-environment-empty">{t(locale.get(), "runtime.environment_unavailable")}</div>
                            }.into_view();
                        }
                        if state.loading && state.snapshot.is_none() {
                            return view! {
                                <div class="runtime-environment-empty">{t(locale.get(), "runtime.objects_loading")}</div>
                            }.into_view();
                        }
                        if let Some(error) = state.error {
                            return view! { <div class="context-error">{error}</div> }.into_view();
                        }
                        let Some(snapshot) = state.snapshot else {
                            return view! {
                                <div class="runtime-environment-empty">{t(locale.get(), "runtime.objects_hint")}</div>
                            }.into_view();
                        };
                        if snapshot.objects.is_empty() {
                            return view! {
                                <div class="runtime-environment-empty">{t(locale.get(), "runtime.objects_empty")}</div>
                            }.into_view();
                        }
                        let query = filter_query.get();
                        let objects = snapshot
                            .objects
                            .into_iter()
                            .filter(|object| runtime_object_matches(object, &query))
                            .collect::<Vec<_>>();
                        if objects.is_empty() {
                            return view! {
                                <div class="runtime-environment-empty">{t(locale.get(), "runtime.objects_none_match")}</div>
                            }.into_view();
                        }
                        let shown = objects.len();
                        let total = snapshot.total_count;
                        view! {
                            <div class="runtime-environment-rows">
                                {objects.into_iter().map(|object| {
                                    let is_error = object.type_name.eq_ignore_ascii_case("unavailable");
                                    let size = object.size_bytes.map(format_bytes).unwrap_or_else(|| "—".into());
                                    let summary = if object.summary.is_empty() { "—".into() } else { object.summary };
                                    let quote = runtime_object_quote(
                                        language_label, &object.name, &object.type_name, &summary, &size,
                                    );
                                    let key_quote = quote.clone();
                                    view! {
                                        <div class="runtime-environment-row" class:is-error=is_error
                                            role="button" tabindex="0"
                                            title=t(locale.get(), "runtime.quote_object")
                                            on:click=move |ev: web_sys::MouseEvent| {
                                                selection_popup.set(Some((
                                                    quote.clone(), None, ev.client_x(), ev.client_y(),
                                                )));
                                            }
                                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                if ev.key() != "Enter" && ev.key() != " " {
                                                    return;
                                                }
                                                ev.prevent_default();
                                                let Some(rect) = ev.target()
                                                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                                                    .map(|el| el.get_bounding_client_rect())
                                                else {
                                                    return;
                                                };
                                                selection_popup.set(Some((
                                                    key_quote.clone(), None,
                                                    (rect.left() + rect.width() / 2.0) as i32,
                                                    rect.bottom() as i32,
                                                )));
                                            }>
                                            <span class="runtime-object-name" title=object.name.clone()>{object.name}</span>
                                            <span class="runtime-object-type" title=object.type_name.clone()>{object.type_name}</span>
                                            <span class="runtime-object-value" title=summary.clone()>{summary}</span>
                                            <span class="runtime-object-size">{size}</span>
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                            {(shown < total || !query.trim().is_empty()).then(|| view! {
                                <div class="runtime-objects-limit">{
                                    tf(locale.get(), "runtime.objects_showing", &[
                                        ("shown", &shown.to_string()),
                                        ("total", &total.to_string()),
                                    ])
                                }</div>
                            })}
                        }.into_view()
                    }}
                </div>
            </section>
        }
    })
    }
}

#[component]
pub(crate) fn RuntimeCard(
    runtime_slot: RuntimeSlot,
    interpreter_form: Option<RuntimeInterpreterForm>,
    runtime_interpreter_form: RwSignal<Option<RuntimeInterpreterForm>>,
    runtime_environment: RwSignal<Option<RuntimeSlot>>,
    locale: RwSignal<Locale>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    object_states: RwSignal<HashMap<String, RuntimeObjectState>>,
) -> impl IntoView {
    let slot = runtime_slot;
    let status = slot
        .info
        .as_ref()
        .map(|info| info.status.clone())
        .unwrap_or_else(|| {
            if slot.available {
                "missing".into()
            } else {
                "unavailable".into()
            }
        });
    let status_class = format!("runtime-status {status}");
    let language_label = if slot.language == "r" { "R" } else { "Python" };
    let identity = format!("{} · {}", slot.project_label, slot.context_label);
    let metadata = slot.info.as_ref().map(|info| {
        let mut parts = Vec::new();
        if let Some(interpreter) = info.interpreter.as_deref() {
            parts.push(interpreter.to_string());
        }
        if let Some(version) = info.version.as_deref() {
            parts.push(version.to_string());
        }
        if let Some(pid) = info.process_id {
            parts.push(format!("PID {pid}"));
        }
        parts.join(" · ")
    });
    let details = slot.info.as_ref().map(|info| {
        let activity =
            format_relative_time(info.last_activity_at_ms as i64, locale.get_untracked());
        let started = format_relative_time(info.started_at_ms as i64, locale.get_untracked());
        let memory = info
            .resident_memory_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "—".into());
        format!(
            "{} {} · {} {} · {} {} · {} {}",
            t(locale.get_untracked(), "runtime.generation"),
            info.generation,
            t(locale.get_untracked(), "runtime.memory"),
            memory,
            t(locale.get_untracked(), "runtime.started"),
            started,
            t(locale.get_untracked(), "runtime.last_activity"),
            activity
        )
    });
    let runtime_id = slot
        .info
        .as_ref()
        .map(|info| info.runtime_id.clone())
        .unwrap_or_default();
    let last_error = slot.info.as_ref().and_then(|info| info.last_error.clone());

    let start_context = slot.context_id.clone();
    let start_language = slot.language.clone();
    let stop_project = slot.project_id.clone();
    let stop_context = slot.context_id.clone();
    let stop_language = slot.language.clone();
    let restart_project = slot.project_id.clone();
    let restart_context = slot.context_id.clone();
    let restart_language = slot.language.clone();
    let environment_slot = slot.clone();
    let selected_project = slot.project_id.clone();
    let selected_context = slot.context_id.clone();
    let selected_language = slot.language.clone();
    let can_stop = matches!(status.as_str(), "starting" | "ready" | "busy");
    let can_restart = matches!(status.as_str(), "ready" | "busy" | "dead");
    let can_start = status == "missing";

    view! {
        <div class="runtime-card" data-runtime-language=slot.language.clone()
            class:environment-active=move || runtime_environment.with(|selected| {
                selected.as_ref().is_some_and(|selected| {
                    selected.project_id == selected_project
                        && selected.context_id == selected_context
                        && selected.language == selected_language
                })
            })
            data-runtime-context=slot.context_id.clone() data-runtime-id=runtime_id.clone()>
            <div class="runtime-card-head">
                <button type="button" class="runtime-language"
                    aria-label=tf(locale.get_untracked(), "runtime.open_environment", &[("language", language_label)])
                    on:click=move |_| {
                        open_runtime_environment(
                            environment_slot.clone(),
                            runtime_environment,
                            object_states,
                            runtimes,
                            locale,
                        );
                    }>
                    <span>{language_label}</span>
                    <span class="runtime-language-open" aria-hidden="true">{compose_icon("chevron-right")}</span>
                </button>
                <span class=status_class>{runtime_status_label(locale.get_untracked(), &status)}</span>
            </div>
            <div class="runtime-identity">{identity}</div>
            {metadata.filter(|value| !value.is_empty()).map(|value| view! {
                <div class="runtime-meta">{value}</div>
            })}
            {details.map(|value| view! { <div class="runtime-details">{value}</div> })}
            {(status == "unavailable").then(|| view! {
                <div class="runtime-unavailable">{t(locale.get_untracked(), "runtime.unavailable_hint")}</div>
            })}
            {last_error.map(|error| view! { <div class="context-error">{error}</div> })}
            <div class="runtime-actions">
                {interpreter_form.map(|form| view! {
                    <button type="button" class="runtime-config"
                        on:click=move |_| runtime_interpreter_form.set(Some(form.clone()))>
                        {move || t(locale.get(), "runtime.configure")}
                    </button>
                })}
                {can_start.then(|| view! {
                    <button type="button" class="runtime-start" on:click=move |_| {
                        invoke_runtime_control(
                            "start_runtime",
                            serde_json::json!({
                                "contextId": start_context.clone(),
                                "language": start_language.clone(),
                            }),
                            locale,
                            runtimes,
                        );
                    }>{move || t(locale.get(), "runtime.start")}</button>
                })}
                {can_stop.then(|| view! {
                    <button type="button" class="runtime-stop" on:click=move |_| {
                        invoke_runtime_control(
                            "stop_runtime",
                            serde_json::json!({
                                "projectId": stop_project.clone(),
                                "contextId": stop_context.clone(),
                                "language": stop_language.clone(),
                            }),
                            locale,
                            runtimes,
                        );
                    }>{move || t(locale.get(), "runtime.stop")}</button>
                })}
                {can_restart.then(|| view! {
                    <button type="button" class="runtime-restart" on:click=move |_| {
                        invoke_runtime_control(
                            "restart_runtime",
                            serde_json::json!({
                                "projectId": restart_project.clone(),
                                "contextId": restart_context.clone(),
                                "language": restart_language.clone(),
                            }),
                            locale,
                            runtimes,
                        );
                    }>{move || t(locale.get(), "runtime.restart")}</button>
                })}
            </div>
        </div>
    }
}

#[component]
pub(crate) fn SessionRuntimeStrip(
    locale: RwSignal<Locale>,
    execution_contexts: RwSignal<Vec<ExecutionContext>>,
    session_execution_contexts: RwSignal<HashSet<String>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    active_project: RwSignal<Option<ProjectInfo>>,
    projects: RwSignal<Vec<ProjectSummary>>,
    runtime_environment: RwSignal<Option<RuntimeSlot>>,
    runtime_environment_pinned: RwSignal<bool>,
    object_states: RwSignal<HashMap<String, RuntimeObjectState>>,
    context_details_modal: RwSignal<Option<(String, ContextModalKind)>>,
    selected_context_id: RwSignal<Option<String>>,
) -> impl IntoView {
    let groups = create_memo(move |_| {
        session_runtime_strip_view(
            runtime_slots(
                runtimes.get(),
                &execution_contexts.get(),
                active_project.get(),
                &projects.get(),
            ),
            &execution_contexts.get(),
            &session_execution_contexts.get(),
        )
    });
    view! {
        {move || {
            let groups = groups.get();
            if groups.is_empty() {
                return None;
            }
            Some(view! {
                <div class="session-runtime-strip" data-testid="session-runtime-strip"
                    aria-label=t(locale.get(), "runtime.strip_title")>
                    {groups.into_iter().map(|group| {
                        let manage_id = group.context_id.clone();
                        let kind_icon = if group.kind == "local" { "monitor" } else { "server" };
                        view! {
                            <div class="session-runtime-group" data-testid="session-runtime-group"
                                data-runtime-context=group.context_id.clone()>
                                <button type="button" class="session-runtime-host"
                                    title=tf(locale.get(), "runtime.strip_manage", &[("context", &group.context_label)])
                                    aria-label=tf(locale.get(), "runtime.strip_manage", &[("context", &group.context_label)])
                                    on:click=move |_| {
                                        selected_context_id.set(Some(manage_id.clone()));
                                        context_details_modal.set(Some((
                                            manage_id.clone(),
                                            ContextModalKind::Runtimes,
                                        )));
                                    }>
                                    <span class="session-runtime-host-icon">{compose_icon(kind_icon)}</span>
                                    <span class="session-runtime-host-name">{group.context_label.clone()}</span>
                                </button>
                                <div class="session-runtime-chips">
                                    {group.chips.into_iter().map(|chip| {
                                        let language_label = language_display(&chip.language).to_string();
                                        let context_label = chip.context_label.clone();
                                        let context_id = chip.context_id.clone();
                                        let language = chip.language.clone();
                                        let status = chip.status.clone();
                                        let status_class = format!("runtime-status {status}");
                                        view! {
                                            <button type="button" class="session-runtime-chip"
                                                data-testid="session-runtime-chip"
                                                data-runtime-language=chip.language.clone()
                                                data-runtime-context=chip.context_id.clone()
                                                data-runtime-status=status.clone()
                                                title=tf(
                                                    locale.get(),
                                                    "runtime.strip_open",
                                                    &[("language", &language_label), ("context", &context_label)],
                                                )
                                                aria-label=tf(
                                                    locale.get(),
                                                    "runtime.strip_open",
                                                    &[("language", &language_label), ("context", &context_label)],
                                                )
                                                on:click=move |_| {
                                                    let Some(slot) = runtime_slots(
                                                        runtimes.get_untracked(),
                                                        &execution_contexts.get_untracked(),
                                                        active_project.get_untracked(),
                                                        &projects.get_untracked(),
                                                    )
                                                    .into_iter()
                                                    .find(|slot| {
                                                        slot.context_id == context_id
                                                            && slot.language == language
                                                    })
                                                    else {
                                                        return;
                                                    };
                                                    open_runtime_environment(
                                                        slot,
                                                        runtime_environment,
                                                        object_states,
                                                        runtimes,
                                                        locale,
                                                    );
                                                    if !runtime_environment_pinned.get_untracked() {
                                                        selected_context_id.set(Some(context_id.clone()));
                                                        context_details_modal.set(Some((
                                                            context_id.clone(),
                                                            ContextModalKind::Runtimes,
                                                        )));
                                                    }
                                                }>
                                                <span class="session-runtime-lang">{language_label}</span>
                                                <span class=status_class>{runtime_status_label(locale.get(), &status)}</span>
                                            </button>
                                        }
                                    }).collect_view()}
                                </div>
                            </div>
                        }
                    }).collect_view()}
                </div>
            })
        }}
    }
}

/// Artifacts a run produced, as `(artifact id, filename)`.
///
/// ponytail: read off the research graph's `produced` edges rather than through
/// a new `list_run_artifacts` command. `Store::save_run_artifact_link` writes
/// the `run_artifacts` row and this edge in the same call, so the two carry the
/// same set — and `get_research_graph` is already a registered command. Add the
/// dedicated command when a run card needs a column `run_artifacts` has and the
/// graph doesn't (the link `role`, or per-artifact size).
pub(crate) fn run_artifact_links(graph: &ResearchGraph, run_id: &str) -> Vec<(String, String)> {
    let source = format!("run:{run_id}");
    let titles: HashMap<&str, &str> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.title.as_str()))
        .collect();
    graph
        .edges
        .iter()
        .filter(|edge| edge.source_id == source && edge.relation == "produced")
        .filter_map(|edge| {
            let id = edge.target_id.strip_prefix("artifact:")?;
            let title = titles
                .get(edge.target_id.as_str())
                .map(|title| (*title).to_string())
                .unwrap_or_else(|| id.to_string());
            Some((id.to_string(), title))
        })
        .collect()
}

#[cfg(test)]
mod run_artifact_link_tests {
    use super::*;

    fn edge(source: &str, target: &str, relation: &str) -> ResearchEdge {
        ResearchEdge {
            source_id: source.into(),
            target_id: target.into(),
            relation: relation.into(),
            metadata_json: "{}".into(),
        }
    }

    #[test]
    fn keeps_only_this_run_s_produced_artifacts() {
        let graph = ResearchGraph {
            nodes: vec![ResearchNode {
                id: "artifact:a1".into(),
                kind: "artifact".into(),
                title: "table.tsv".into(),
                ref_id: Some("a1".into()),
                metadata_json: "{}".into(),
            }],
            edges: vec![
                edge("run:r1", "artifact:a1", "produced"),
                // Another run's output, and a non-production relation.
                edge("run:r2", "artifact:a2", "produced"),
                edge("run:r1", "decision:d1", "informs"),
            ],
        };
        assert_eq!(
            run_artifact_links(&graph, "r1"),
            vec![("a1".to_string(), "table.tsv".to_string())]
        );
        // An artifact whose node was not returned still opens, under its id.
        assert_eq!(
            run_artifact_links(&graph, "r2"),
            vec![("a2".to_string(), "a2".to_string())]
        );
    }
}

pub(crate) fn run_title(run: &RunSummary) -> String {
    if !run.title.trim().is_empty() {
        run.title.clone()
    } else {
        run.id.clone()
    }
}

pub(crate) fn run_progress(run: &RunSummary) -> Option<RunProgress> {
    serde_json::from_str(&run.progress_json).ok()
}

pub(crate) fn method_search_progress(run: &RunSummary) -> Option<MethodSearchProgressView> {
    (run.kind == "method_search")
        .then(|| serde_json::from_str(&run.progress_json).ok())
        .flatten()
}

pub(crate) fn run_output_preview(run: &RunRecord) -> String {
    // Tails are stored raw; fold `\r` progress-bar frames before slicing lines
    // so a single overwritten line cannot dominate the preview.
    let stdout = run.stdout_tail.as_deref().map(fold_carriage_returns);
    let stderr = run.stderr_tail.as_deref().map(fold_carriage_returns);
    let mut output = match (&stdout, &stderr) {
        (Some(stdout), Some(stderr)) if !stdout.is_empty() && !stderr.is_empty() => {
            format!("{stdout}\n[stderr]\n{stderr}")
        }
        (Some(stdout), _) => stdout.clone(),
        (_, Some(stderr)) => stderr.clone(),
        _ => String::new(),
    };
    let lines = output.lines().collect::<Vec<_>>();
    if lines.len() > 8 {
        output = lines[lines.len() - 8..].join("\n");
    }
    output
}

#[component]
fn RunDetailDisclosure(run_id: String, locale: RwSignal<Locale>) -> impl IntoView {
    let open = create_rw_signal(false);
    let detail = create_rw_signal(None::<RunRecord>);
    let loading = create_rw_signal(false);
    let toggle_id = run_id.clone();
    view! {
        <details class="run-output" open=move || open.get()>
            <summary on:click=move |event| {
                event.prevent_default();
                let next = !open.get_untracked();
                open.set(next);
                if !next || detail.get_untracked().is_some() || loading.get_untracked() {
                    return;
                }
                loading.set(true);
                let run_id = toggle_id.clone();
                spawn_local(async move {
                    let args = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                    if let Ok(value) = invoke_checked("get_run_detail", args).await {
                        if let Ok(record) = serde_wasm_bindgen::from_value::<RunRecord>(value) {
                            detail.set(Some(record));
                        }
                    }
                    loading.set(false);
                });
            }>{move || t(locale.get(), "runs.output")}</summary>
            {move || loading.get().then(|| view! {
                <div class="control-empty">{t(locale.get(), "loading")}</div>
            })}
            {move || detail.get().map(|run| {
                let output = run_output_preview(&run);
                view! {
                    {run.command.filter(|command| !command.trim().is_empty()).map(|command| view! {
                        <div class="run-command">{command}</div>
                    })}
                    {(!output.is_empty()).then(|| view! {
                        <pre data-run-output-for=run.id>{output}</pre>
                    })}
                }
            })}
        </details>
    }
}

const TRANSFER_SETTLED_LINGER_SECONDS: i64 = 3;

pub(crate) fn transfer_progress_visible(
    progress: &RunProgress,
    run_status: &str,
    now: i64,
) -> bool {
    (matches!(run_status, "submitted" | "running" | "cancelling")
        && matches!(progress.phase.as_str(), "uploading" | "downloading"))
        || (now - progress.updated_at).abs() <= TRANSFER_SETTLED_LINGER_SECONDS
}

#[cfg(test)]
mod transfer_progress_tests {
    use super::*;

    fn progress(phase: &str, updated_at: i64) -> RunProgress {
        RunProgress {
            phase: phase.into(),
            direction: "download".into(),
            completed_bytes: 1,
            total_bytes: 1,
            files_completed: 1,
            files_total: 1,
            current_file: None,
            bytes_per_second: None,
            eta_seconds: None,
            updated_at,
        }
    }

    #[test]
    fn active_transfer_stays_visible_even_when_its_progress_timestamp_is_stale() {
        assert!(transfer_progress_visible(
            &progress("downloading", 10),
            "running",
            100,
        ));
    }

    #[test]
    fn settled_transfer_expires_after_the_short_confirmation_window() {
        let transfer = progress("downloaded", 100);
        assert!(transfer_progress_visible(&transfer, "succeeded", 103));
        assert!(!transfer_progress_visible(&transfer, "succeeded", 104));
    }
}

fn transfer_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

pub(crate) fn transfer_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, seconds % 3600 / 60)
    }
}

pub(crate) fn run_status_label(locale: Locale, status: &str) -> String {
    let key = match status {
        "draft" => "runs.status.draft",
        "submitted" => "runs.status.submitted",
        "running" => "runs.status.running",
        "paused" => "runs.status.paused",
        "cancelling" => "runs.status.cancelling",
        "succeeded" => "runs.status.succeeded",
        "failed" => "runs.status.failed",
        "timed_out" => "runs.status.timed_out",
        "cancelled" => "runs.status.cancelled",
        "lost" => "runs.status.lost",
        _ => return status.to_string(),
    };
    t(locale, key).to_string()
}

pub(crate) fn run_progress_meter(progress: RunProgress, locale: Locale) -> impl IntoView {
    let percent = if progress.total_bytes == 0 {
        0
    } else {
        progress
            .completed_bytes
            .saturating_mul(100)
            .div_ceil(progress.total_bytes)
            .min(100)
    };
    let phase_key = match progress.phase.as_str() {
        "uploading" => "transfer.uploading",
        "uploaded" => "transfer.uploaded",
        "downloading" => "transfer.downloading",
        "downloaded" => "transfer.downloaded",
        "cancelled" => "transfer.cancelled",
        "failed" => "transfer.failed",
        _ => "transfer.transferring",
    };
    let bytes = format!(
        "{} / {} · {percent}%",
        transfer_bytes(progress.completed_bytes),
        transfer_bytes(progress.total_bytes)
    );
    let speed = progress
        .bytes_per_second
        .map(|rate| format!("{}/s", transfer_bytes(rate)));
    let eta = progress.eta_seconds.map(|seconds| {
        tf(
            locale,
            "transfer.eta",
            &[("time", &transfer_duration(seconds))],
        )
    });
    let files = (progress.files_total > 1).then(|| {
        tf(
            locale,
            "transfer.files",
            &[
                ("done", &progress.files_completed.to_string()),
                ("total", &progress.files_total.to_string()),
            ],
        )
    });
    view! {
        <div class="run-progress" data-direction=progress.direction>
            <div class="run-progress-head">
                <strong>{t(locale, phase_key)}</strong>
                {progress.current_file.map(|file| view! { <span>{file}</span> })}
            </div>
            <progress max="100" value=percent.to_string()
                aria-label=t(locale, phase_key)></progress>
            <div class="run-progress-meta">
                <span>{bytes}</span>
                {speed.map(|value| view! { <span>{value}</span> })}
                {eta.map(|value| view! { <span>{value}</span> })}
                {files.map(|value| view! { <span>{value}</span> })}
            </div>
        </div>
    }
}

fn load_method_search_details(
    run_id: String,
    details: RwSignal<Option<MethodSearchRunDetails>>,
    loading: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) {
    loading.set(true);
    error.set(None);
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
        match invoke_checked("get_method_search_run", args).await {
            Ok(value) => match serde_wasm_bindgen::from_value(value) {
                Ok(value) => details.set(Some(value)),
                Err(parse_error) => error.set(Some(parse_error.to_string())),
            },
            Err(invoke_error) => error.set(Some(js_error_text(invoke_error))),
        }
        loading.set(false);
    });
}

fn control_method_search(
    command: &'static str,
    run_id: String,
    details: RwSignal<Option<MethodSearchRunDetails>>,
    loading: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) {
    loading.set(true);
    error.set(None);
    spawn_local(async move {
        let args = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
        match invoke_checked(command, args).await {
            Ok(value) => match serde_wasm_bindgen::from_value(value) {
                Ok(value) => details.set(Some(value)),
                Err(parse_error) => error.set(Some(parse_error.to_string())),
            },
            Err(invoke_error) => error.set(Some(js_error_text(invoke_error))),
        }
        loading.set(false);
    });
}

#[component]
fn MethodSearchRunPanel(
    run_id: String,
    locale: RwSignal<Locale>,
    modal: RwSignal<Option<(String, ContextModalKind)>>,
    modal_artifact: RwSignal<Option<ModalArtifact>>,
) -> impl IntoView {
    let expanded = create_rw_signal(false);
    let loading = create_rw_signal(false);
    let details = create_rw_signal(None::<MethodSearchRunDetails>);
    let error = create_rw_signal(None::<String>);
    let inspect_run_id = run_id.clone();
    let refresh_run_id = run_id.clone();
    view! {
        <div class="method-search-panel" data-testid="method-search-panel">
            <button type="button" class="secondary method-search-inspect"
                data-testid="method-search-inspect"
                aria-expanded=move || expanded.get().to_string()
                on:click=move |_| {
                    let next = !expanded.get_untracked();
                    expanded.set(next);
                    if next && details.get_untracked().is_none() {
                        load_method_search_details(
                            inspect_run_id.clone(),
                            details,
                            loading,
                            error,
                        );
                    }
                }>
                {move || if expanded.get() {
                    t(locale.get(), "method_search.hide")
                } else {
                    t(locale.get(), "method_search.inspect")
                }}
            </button>
            {move || {
                let current_refresh_id = refresh_run_id.clone();
                let current_run_id = run_id.clone();
                expanded.get().then(move || view! {
                <section class="method-search-details" data-testid="method-search-details">
                    <div class="method-search-detail-head">
                        <strong>{move || t(locale.get(), "method_search.contract")}</strong>
                        <button type="button" class="icon-btn"
                            data-testid="method-search-refresh"
                            title=move || t(locale.get(), "runs.refresh")
                            aria-label=move || t(locale.get(), "runs.refresh")
                            disabled=move || loading.get()
                            on:click=move |_| load_method_search_details(
                                current_refresh_id.clone(),
                                details,
                                loading,
                                error,
                            )>{compose_icon("sync")}</button>
                    </div>
                    {move || loading.get().then(|| view! {
                        <div class="method-search-loading">{t(locale.get(), "method_search.loading")}</div>
                    })}
                    {move || error.get().map(|message| view! {
                        <div class="context-error" data-testid="method-search-error">{message}</div>
                    })}
                    {move || {
                        let command_run_id = current_run_id.clone();
                        details.get().map(move |detail| {
                        let status = detail.run.status.clone();
                        let spec = detail.spec.clone();
                        let audit = detail.audit.clone();
                        let state = detail.state.clone();
                        let candidates = detail.candidates.clone();
                        let strategies = detail.strategies.clone();
                        let outputs = detail.outputs.clone();
                        let progress: MethodSearchProgressView =
                            serde_json::from_str(&detail.run.progress_json).unwrap_or_default();
                        let start_id = command_run_id.clone();
                        let pause_id = command_run_id.clone();
                        let resume_id = command_run_id.clone();
                        let cancel_id = command_run_id;
                        view! {
                            <div class="method-search-approval" data-testid="method-search-contract">
                                <div class="method-search-status-row">
                                    <span class=format!("run-status {status}")>
                                        {run_status_label(locale.get(), &status)}
                                    </span>
                                    <code title=state.spec_artifact_version_id.clone()>
                                        {format!("spec {}", state.spec_artifact_version_id.chars().take(12).collect::<String>())}
                                    </code>
                                    <code title=detail.audit_artifact_version_id.clone()>
                                        {format!("audit {}", detail.audit_artifact_version_id.chars().take(12).collect::<String>())}
                                    </code>
                                </div>
                                <p>{spec.objective.clone()}</p>
                                <dl class="method-search-contract-grid">
                                    <div><dt>{t(locale.get(), "method_search.target")}</dt><dd><code>{format!("{}::{}", spec.target.source_path, spec.target.symbol)}</code></dd></div>
                                    <div><dt>{t(locale.get(), "method_search.evaluator")}</dt><dd><code>{spec.evaluator.entry_path}</code></dd></div>
                                    <div><dt>{t(locale.get(), "method_search.metric")}</dt><dd>{format!("{} · {}", spec.metrics.primary, spec.metrics.direction)}</dd></div>
                                    <div><dt>{t(locale.get(), "method_search.context")}</dt><dd><code>{detail.run.context_id}</code></dd></div>
                                    <div><dt>{t(locale.get(), "method_search.baseline")}</dt><dd>{format!("{:.6} ± {:.6}", audit.baseline.median_primary, audit.baseline.spread)}</dd></div>
                                    <div><dt>{t(locale.get(), "method_search.noise")}</dt><dd>{format!("{:.6}", audit.baseline.noise_floor)}</dd></div>
                                    <div><dt>{t(locale.get(), "method_search.reachability")}</dt><dd>{if audit.sentinel_reachable { "✓" } else { "×" }}</dd></div>
                                    <div><dt>{t(locale.get(), "method_search.budget")}</dt><dd>{format!("{} · {} · {}s · eval {}s", spec.budget.max_candidates, spec.budget.max_cost_microunits, spec.budget.max_wall_seconds, spec.budget.max_evaluator_seconds)}</dd></div>
                                    <div><dt>{t(locale.get(), "method_search.protected")}</dt><dd>{spec.protected_paths.len().to_string()}</dd></div>
                                </dl>
                                {(!spec.metrics.guardrails.is_empty()).then(|| view! {
                                    <div class="method-search-tags">
                                        <span>{t(locale.get(), "method_search.guardrails")}</span>
                                        {spec.metrics.guardrails.into_iter().map(|guardrail| view! {
                                            <code>{format!("{} {} {}", guardrail.metric, guardrail.op, guardrail.value)}</code>
                                        }).collect_view()}
                                    </div>
                                })}
                                {(!spec.constraints.is_empty()).then(|| view! {
                                    <ul class="method-search-findings">
                                        {spec.constraints.into_iter().map(|finding| view! { <li>{finding}</li> }).collect_view()}
                                    </ul>
                                })}
                                <div class="method-search-actions">
                                    {(status == "draft").then(|| view! {
                                        <button type="button" class="primary"
                                            data-testid="method-search-start"
                                            disabled=move || loading.get()
                                            on:click=move |_| control_method_search(
                                                "start_method_search",
                                                start_id.clone(), details, loading, error,
                                            )>{move || t(locale.get(), "method_search.start")}</button>
                                    })}
                                    {matches!(status.as_str(), "submitted" | "running").then(|| view! {
                                        <button type="button" class="secondary"
                                            data-testid="method-search-pause"
                                            disabled=move || loading.get()
                                            on:click=move |_| control_method_search(
                                                "pause_method_search",
                                                pause_id.clone(), details, loading, error,
                                            )>{move || t(locale.get(), "method_search.pause")}</button>
                                    })}
                                    {(status == "paused").then(|| view! {
                                        <button type="button" class="primary"
                                            data-testid="method-search-resume"
                                            disabled=move || loading.get()
                                            on:click=move |_| control_method_search(
                                                "resume_method_search",
                                                resume_id.clone(), details, loading, error,
                                            )>{move || t(locale.get(), "method_search.resume")}</button>
                                    })}
                                    {matches!(status.as_str(), "draft" | "submitted" | "running" | "paused").then(|| view! {
                                        <button type="button" class="agents-danger"
                                            data-testid="method-search-cancel"
                                            disabled=move || loading.get()
                                            on:click=move |_| control_method_search(
                                                "cancel_method_search",
                                                cancel_id.clone(), details, loading, error,
                                            )>{move || t(locale.get(), "method_search.cancel")}</button>
                                    })}
                                </div>
                            </div>
                            <div class="method-search-progress-grid" data-testid="method-search-progress">
                                <div><span>{t(locale.get(), "method_search.phase")}</span><strong>{progress.phase}</strong></div>
                                <div><span>{t(locale.get(), "method_search.baseline")}</span><strong>{progress.baseline_primary.map(|value| format!("{value:.6}")).unwrap_or_else(|| "—".into())}</strong></div>
                                <div><span>{t(locale.get(), "method_search.best")}</span><strong>{progress.best_primary.map(|value| format!("{value:.6}")).unwrap_or_else(|| "—".into())}</strong></div>
                                <div><span>{t(locale.get(), "method_search.candidates")}</span><strong>{progress.candidate_count}</strong></div>
                                <div><span>{t(locale.get(), "method_search.success_failed")}</span><strong>{format!("{} / {}", progress.successful_count, progress.failed_count)}</strong></div>
                                <div><span>{t(locale.get(), "method_search.cost")}</span><strong>{progress.cost_microunits}</strong></div>
                                <div><span>{t(locale.get(), "method_search.strategy")}</span><strong>{progress.current_strategy.unwrap_or_else(|| "—".into())}</strong></div>
                                <div><span>{t(locale.get(), "method_search.checkpoint")}</span><strong>{progress.last_checkpoint_at.map(|value| format!("{}s", ((js_sys::Date::now() / 1000.0) as i64).saturating_sub(value))).unwrap_or_else(|| "—".into())}</strong></div>
                                <div><span>{t(locale.get(), "method_search.best_candidate")}</span><strong>{progress.best_candidate_id.map(|value| value.chars().take(12).collect::<String>()).unwrap_or_else(|| "—".into())}</strong></div>
                            </div>
                            {(!outputs.is_empty()).then(|| view! {
                                <div class="method-search-outputs" data-testid="method-search-outputs">
                                    <strong>{t(locale.get(), "method_search.outputs")}</strong>
                                    {outputs.into_iter().map(|output| {
                                        let name = if output.logical_output_key.trim().is_empty() {
                                            output.role.clone()
                                        } else {
                                            output.logical_output_key.clone()
                                        };
                                        let target = (
                                            format!("artifact-version:{}", output.artifact_version_id),
                                            output.source_path.clone(),
                                            file_kind(&output.source_path).unwrap_or("text").to_string(),
                                        );
                                        view! {
                                            <button type="button" class="run-artifact"
                                                on:click=move |_| {
                                                    modal.set(None);
                                                    modal_artifact.set(Some(target.clone()));
                                                }>{name}</button>
                                        }
                                    }).collect_view()}
                                </div>
                            })}
                            {(!candidates.is_empty()).then(|| view! {
                                <details class="method-search-lineage" data-testid="method-search-lineage">
                                    <summary>{tf(locale.get(), "method_search.lineage", &[("count", &candidates.len().to_string())])}</summary>
                                    <div>
                                        {candidates.into_iter().map(|candidate| view! {
                                            <article data-candidate-id=candidate.id>
                                                <span>{format!("#{}", candidate.sequence)}</span>
                                                <code>{candidate.family}</code>
                                                <code>{candidate.strategy_key}</code>
                                                <strong>{candidate.primary_score.map(|value| format!("{value:.6}")).unwrap_or_else(|| candidate.status.clone())}</strong>
                                                {candidate.changed_lines.map(|value| view! { <small>{format!("Δ {value}")}</small> })}
                                                {candidate.runtime_ms.map(|value| view! { <small>{format!("{value} ms")}</small> })}
                                                {candidate.parent_candidate_id.map(|value| view! { <small>{format!("← {}", value.chars().take(8).collect::<String>())}</small> })}
                                                {candidate.rationale.map(|value| view! { <p>{value}</p> })}
                                                {candidate.error.map(|value| view! { <p class="context-error">{value}</p> })}
                                            </article>
                                        }).collect_view()}
                                    </div>
                                </details>
                            })}
                            {(!strategies.is_empty()).then(|| view! {
                                <details class="method-search-lineage">
                                    <summary>{t(locale.get(), "method_search.strategies")}</summary>
                                    <div>
                                        {strategies.into_iter().map(|strategy| view! {
                                            <article>
                                                <code>{strategy.strategy_key}</code>
                                                <span>{strategy.category}</span>
                                                <strong>{format!("{:.3}", strategy.weight)}</strong>
                                                <small>{format!("{} / {}", strategy.improvements, strategy.attempts)}</small>
                                            </article>
                                        }).collect_view()}
                                    </div>
                                </details>
                            })}
                            <div class="method-search-integrity">
                                <code title=state.spec_sha256>{state.control_state}</code>
                                <span>{state.result_status.unwrap_or_else(|| t(locale.get(), "method_search.pending").into())}</span>
                                <span>{format!("{} / {}", audit.baseline.successful_repetitions, audit.baseline.repetitions)}</span>
                                <span>{format!("{:.1}%", audit.baseline.failure_rate * 100.0)}</span>
                                <span>{format!("{} {}", audit.protected_files.len(), t(locale.get(), "method_search.protected"))}</span>
                                <span>{format!("{} {}", audit.findings.len(), t(locale.get(), "method_search.findings"))}</span>
                                <code title=audit.target_source_sha256>{spec.target.source_artifact_version_id.chars().take(12).collect::<String>()}</code>
                                <code title=spec.evaluator.artifact_version_id>{format!("{}×{}s", spec.evaluator.repetitions, spec.evaluator.timeout_seconds)}</code>
                                <span>{if spec.final_verification.is_some() { t(locale.get(), "method_search.final_enabled") } else { t(locale.get(), "method_search.validation_only") }}</span>
                            </div>
                        }.into_view()
                    })}}
                </section>
            })}}
        </div>
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextModalKind {
    Machine,
    Runtimes,
    Runs,
    RemoteFiles,
}

/// Files this project ledgered on one server, with delete actions for
/// retracted/replaced entries. Fetched on open; never persisted client-side.
#[component]
fn RemoteFilesPane(context_id: String, locale: RwSignal<Locale>) -> impl IntoView {
    let files = create_rw_signal(Vec::<RemoteFileView>::new());
    let error = create_rw_signal(None::<String>);
    let fetch_context_id = context_id.clone();
    let fetch = move || {
        let context_id = fetch_context_id.clone();
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "contextId": context_id })).unwrap();
            match invoke_checked("list_remote_files", args).await {
                Ok(value) => {
                    if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<RemoteFileView>>(value) {
                        files.set(list);
                    }
                }
                Err(value) => error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(value),
                ))),
            }
        });
    };
    fetch.clone()();
    let section_context_id = context_id.clone();
    view! {
        <section class="control-section context-modal-section remote-files-pane"
            data-context-id=section_context_id data-testid="remote-files-pane">
            <div class="control-section-head">
                <span>{t(locale.get(), "contexts.remote_files")}</span>
                <span class="control-count">{move || files.get().len().to_string()}</span>
            </div>
            {move || error.get().map(|message| view! {
                <div class="context-error">{message}</div>
            })}
            {move || {
                let rows = files.get();
                if rows.is_empty() {
                    view! { <div class="control-empty">{t(locale.get(), "remote_files.empty")}</div> }.into_view()
                } else {
                    rows.into_iter().map(|file| {
                        let state_label = t(locale.get(), &format!("remote_files.state_{}", file.state));
                        let state_class = format!("remote-file-state {}", file.state);
                        let deletable = file.state != "active";
                        let delete_context_id = context_id.clone();
                        let delete_id = file.id.clone();
                        let refetch = fetch.clone();
                        view! {
                            <div class="remote-file-row" data-testid="remote-file-row">
                                <div class="remote-file-main">
                                    <code class="remote-file-path">{file.remote_path.clone()}</code>
                                    <div class="remote-file-meta">
                                        <span class=state_class>{state_label}</span>
                                        <span>{t(locale.get(), &format!("remote_files.source_{}", file.source))}</span>
                                        {file.run_status.map(|status| view! { <span>{status}</span> })}
                                    </div>
                                </div>
                                {deletable.then(|| view! {
                                    <button type="button" class="icon-btn remote-file-delete"
                                        title=t(locale.get(), "remote_files.delete")
                                        aria-label=t(locale.get(), "remote_files.delete")
                                        on:click=move |_| {
                                            let context_id = delete_context_id.clone();
                                            let id = delete_id.clone();
                                            let refetch = refetch.clone();
                                            spawn_local(async move {
                                                let args = to_value(&serde_json::json!({
                                                    "contextId": context_id,
                                                    "ids": [id],
                                                }))
                                                .unwrap();
                                                match invoke_checked("remove_remote_files", args).await {
                                                    Ok(_) => show_toast(&t(locale.get_untracked(), "remote_files.deleted")),
                                                    Err(value) => show_toast(&localize_backend(
                                                        locale.get_untracked(),
                                                        &js_error_text(value),
                                                    )),
                                                }
                                                refetch();
                                            });
                                        }>{compose_icon("trash")}</button>
                                })}
                            </div>
                        }.into_view()
                    }).collect_view()
                }
            }}
        </section>
    }
}

#[component]
pub(crate) fn ContextDetailsOverlay(
    modal: RwSignal<Option<(String, ContextModalKind)>>,
    runtime_environment: RwSignal<Option<RuntimeSlot>>,
    runtime_environment_pinned: RwSignal<bool>,
    runtime_environment_position: RwSignal<(i32, i32)>,
    contexts: RwSignal<Vec<ExecutionContext>>,
    runtimes: RwSignal<Vec<RuntimeInfo>>,
    runs: RwSignal<Vec<RunSummary>>,
    research_graph: RwSignal<ResearchGraph>,
    modal_artifact: RwSignal<Option<ModalArtifact>>,
    active_project: RwSignal<Option<ProjectInfo>>,
    projects: RwSignal<Vec<ProjectSummary>>,
    runtime_interpreter_form: RwSignal<Option<RuntimeInterpreterForm>>,
    object_states: RwSignal<HashMap<String, RuntimeObjectState>>,
    locale: RwSignal<Locale>,
    selection_popup: RwSignal<Option<(String, Option<String>, i32, i32)>>,
    on_use_in_publication: Callback<PublicationEvidenceSource>,
) -> impl IntoView {
    // Run→artifact links live on the graph's `produced` edges, so the Runs view
    // needs a fresh graph each time it opens.
    create_effect(move |_| {
        if matches!(modal.get(), Some((_, ContextModalKind::Runs))) {
            crate::research::refresh_research_graph(research_graph);
        }
    });
    create_effect(move |_| {
        let active = modal.get();
        let pinned = runtime_environment_pinned.get();
        let should_close = runtime_environment.with_untracked(|selected| {
            !pinned && selected.as_ref().is_some_and(|slot| {
                !matches!(&active, Some((context_id, ContextModalKind::Runtimes)) if context_id == &slot.context_id)
            })
        });
        if should_close {
            runtime_environment.set(None);
        }
    });

    move || {
        let Some((context_id, kind)) = modal.get() else {
            return ().into_view();
        };
        let Some(context) = contexts
            .get()
            .into_iter()
            .find(|context| context.id == context_id)
        else {
            modal.set(None);
            return ().into_view();
        };
        let context_label = if context.label.trim().is_empty() {
            context.id.clone()
        } else {
            context.label.clone()
        };
        let title = match kind {
            ContextModalKind::Machine => t(locale.get(), "contexts.machine_info"),
            ContextModalKind::Runtimes => t(locale.get(), "contexts.runtimes"),
            ContextModalKind::Runs => t(locale.get(), "contexts.runs"),
            ContextModalKind::RemoteFiles => t(locale.get(), "contexts.remote_files"),
        };
        let body_context_id = context.id.clone();

        view! {
            <div class="overlay context-details-overlay" role="presentation">
                <div class="modal context-details-modal"
                    class:runtime-details=kind == ContextModalKind::Runtimes
                    class:runs-details=kind == ContextModalKind::Runs
                    class:environment-open=move || {
                        runtime_environment.get().is_some() && !runtime_environment_pinned.get()
                    }
                    role="dialog" aria-modal="true" aria-label=title.clone()>
                    <div class="ps-head">
                        <div class="context-modal-title">
                            <h2>{title}</h2>
                            <span>{context_label}</span>
                        </div>
                        <button type="button" class="ps-close"
                            title=t(locale.get(), "contexts.close_details")
                            aria-label=t(locale.get(), "contexts.close_details")
                            on:click=move |_| modal.set(None)>{compose_icon("close")}</button>
                    </div>
                    {match kind {
                        ContextModalKind::RemoteFiles => view! {
                            <RemoteFilesPane context_id=body_context_id.clone() locale=locale />
                        }.into_view(),
                        ContextModalKind::Machine => {
                            let status = context.last_probe_status.clone().unwrap_or_else(|| "unknown".into());
                            let status_class = format!("context-status {status}");
                            let error = context.last_probe_error.clone();
                            view! {
                                <div class="context-machine-summary" data-context-id=context.id.clone()>
                                    <div class="context-machine-heading">
                                        <span class="context-id">{context.id.clone()}</span>
                                        <span class=status_class>{status}</span>
                                    </div>
                                    <dl class="context-machine-fields">
                                        <div><dt>{t(locale.get(), "contexts.kind")}</dt><dd>{context.kind.clone()}</dd></div>
                                        <div><dt>{t(locale.get(), "contexts.capabilities")}</dt><dd>{context_capability_summary(&context)}</dd></div>
                                    </dl>
                                    {error.map(|error| view! { <div class="context-error">{error}</div> })}
                                </div>
                            }.into_view()
                        }
                        ContextModalKind::Runtimes => {
                            let section_context_id = body_context_id.clone();
                            view! {
                                <div class="runtime-modal-body">
                                    {move || {
                                        let all_contexts = contexts.get();
                                        let rows = runtime_slots(
                                            runtimes.get(),
                                            &all_contexts,
                                            active_project.get(),
                                            &projects.get(),
                                        ).into_iter()
                                            .filter(|slot| slot.context_id == section_context_id)
                                            .collect::<Vec<_>>();
                                        view! {
                                            <section class="control-section context-modal-section" data-context-id=section_context_id.clone()>
                                                <div class="control-section-head">
                                                    <span>{t(locale.get(), "contexts.runtimes")}</span>
                                                    <div class="control-head-actions">
                                                        <span class="control-count">{rows.len().to_string()}</span>
                                                        <button type="button" class="icon-btn control-refresh"
                                                            title=t(locale.get(), "runtime.refresh")
                                                            aria-label=t(locale.get(), "runtime.refresh")
                                                            on:click=move |_| refresh_runtimes(runtimes)>{compose_icon("sync")}</button>
                                                    </div>
                                                </div>
                                                <div class="runtime-warning">{t(locale.get(), "runtime.state_warning")}</div>
                                                {if rows.is_empty() {
                                                    view! { <div class="control-empty">{t(locale.get(), "runtime.empty")}</div> }.into_view()
                                                } else {
                                                    rows.into_iter().map(|slot| {
                                                        let interpreter_form = all_contexts.iter()
                                                            .find(|context| context.id == slot.context_id)
                                                            .map(RuntimeInterpreterForm::from_context);
                                                        view! {
                                                            <RuntimeCard runtime_slot=slot interpreter_form=interpreter_form
                                                                runtime_interpreter_form=runtime_interpreter_form
                                                                runtime_environment=runtime_environment locale=locale runtimes=runtimes
                                                                object_states=object_states />
                                                        }
                                                    }).collect_view()
                                                }}
                                            </section>
                                        }
                                    }}
                                    {move || (!runtime_environment_pinned.get()).then(|| view! {
                                        <RuntimeEnvironmentPanel selected=runtime_environment
                                            pinned=runtime_environment_pinned
                                            position=runtime_environment_position context_modal=modal
                                            locale=locale states=object_states runtimes=runtimes
                                            contexts=contexts active_project=active_project projects=projects
                                            selection_popup=selection_popup />
                                    })}
                                </div>
                            }.into_view()
                        }
                        ContextModalKind::Runs => {
                            let section_context_id = body_context_id.clone();
                            view! {
                                {move || {
                                    let rows = runs.with(|records| {
                                        records
                                            .iter()
                                        .filter(|run| run.context_id == section_context_id)
                                            .cloned()
                                            .collect::<Vec<_>>()
                                    });
                                    view! {
                                        <section class="control-section context-modal-section" data-context-id=section_context_id.clone()>
                                            <div class="control-section-head">
                                                <span>{t(locale.get(), "contexts.runs")}</span>
                                                <div class="control-head-actions">
                                                    <span class="control-count">{rows.len().to_string()}</span>
                                                    <button type="button" class="icon-btn control-refresh"
                                                        title=t(locale.get(), "runs.refresh")
                                                        aria-label=t(locale.get(), "runs.refresh")
                                                        on:click=move |_| {
                                                            refresh_runs(runs, locale);
                                                            crate::research::refresh_research_graph(research_graph);
                                                        }>{compose_icon("sync")}</button>
                                                </div>
                                            </div>
                                            {if rows.is_empty() {
                                                view! { <div class="control-empty">{t(locale.get(), "runs.empty")}</div> }.into_view()
                                            } else {
                                                rows.into_iter().map(|run| {
                                            let title = run_title(&run);
                                            let status_class = format!("run-status {}", run.status);
                                            let cancel_id = run.id.clone();
                                            let method_search = run.kind == "method_search";
                                            let cancellable = !method_search
                                                && matches!(
                                                    run.status.as_str(),
                                                    "submitted" | "running" | "cancelling"
                                                );
                                            let terminal = matches!(
                                                run.status.as_str(),
                                                "succeeded" | "failed" | "cancelled" | "timed_out" | "lost"
                                            );
                                            let cleaned = run.cleaned_at.is_some();
                                            let cleanable = !method_search
                                                && terminal
                                                && !cleaned
                                                && run.kind != "file_transfer"
                                                && run.remote_workdir.is_some();
                                            let cleanup_id = run.id.clone();
                                            let cleanup_error = run.cleanup_error.clone();
                                            let cancel_label = if run.status == "cancelling" {
                                                t(locale.get(), "runs.force_cancel")
                                            } else {
                                                t(locale.get(), "runs.cancel")
                                            };
                                            let remote_workdir = run.remote_workdir.clone();
                                            let poll_error = run.last_poll_error.clone();
                                            let progress = (!method_search)
                                                .then(|| run_progress(&run))
                                                .flatten();
                                            let method_progress = method_search_progress(&run);
                                            let method_run_id = run.id.clone();
                                            let detail_run_id = run.id.clone();
                                            let meta = match run.exit_code {
                                                Some(code) => format!("{} · {} · exit {code}", run.context_id, run.kind),
                                                None => format!("{} · {}", run.context_id, run.kind),
                                            };
                                            let produced = run_artifact_links(&research_graph.get(), &run.id);
                                            let publication_source = PublicationEvidenceSource {
                                                kind: "run",
                                                id: run.id.clone(),
                                                label: title.clone(),
                                            };
                                            view! {
                                                <div class="run-card" class:method-search=method_search>
                                                    <div class="run-card-head">
                                                        <div class="run-card-main">
                                                            <div class="run-card-title-row">
                                                                <span class="run-title" title=title.clone()>{title}</span>
                                                                <span class=status_class>{run.status.clone()}</span>
                                                            </div>
                                                            <div class="run-meta">{meta}</div>
                                                        </div>
                                                        <div class="run-card-actions">
                                                            <button type="button" class="run-use-publication"
                                                                on:click=move |_| {
                                                                    on_use_in_publication.call(publication_source.clone());
                                                                }>
                                                                {t(locale.get(), "publication.use")}
                                                            </button>
                                                            {cancellable.then(|| {
                                                                let run_id = cancel_id.clone();
                                                                let tip = cancel_label.clone();
                                                                view! {
                                                                    <button type="button" class="icon-btn run-cancel"
                                                                        title=tip.clone()
                                                                        aria-label=tip
                                                                        on:click=move |_| {
                                                                            let run_id = run_id.clone();
                                                                            spawn_local(async move {
                                                                                let arg = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                                                                                let _ = invoke("cancel_run", arg).await;
                                                                                refresh_runs(runs, locale);
                                                                            });
                                                                        }>{compose_icon("close")}</button>
                                                                }
                                                            })}
                                                            {cleanable.then(|| {
                                                                let review_id = cleanup_id.clone();
                                                                let tip = t(locale.get(), "run_review.open");
                                                                let review_modal = use_context::<crate::overlays::RunReviewModal>();
                                                                review_modal.map(|review_modal| view! {
                                                                    <button type="button" class="icon-btn run-review-open"
                                                                        data-testid="run-review-open"
                                                                        title=tip.clone()
                                                                        aria-label=tip
                                                                        on:click=move |_| review_modal.0.set(Some(review_id.clone()))
                                                                    >{compose_icon("folder")}</button>
                                                                })
                                                            })}
                                                            {cleanable.then(|| {
                                                                let run_id = cleanup_id.clone();
                                                                let tip = t(locale.get(), "runs.cleanup");
                                                                view! {
                                                                    <button type="button" class="icon-btn run-cleanup"
                                                                        title=tip.clone()
                                                                        aria-label=tip
                                                                        on:click=move |_| {
                                                                            let run_id = run_id.clone();
                                                                            spawn_local(async move {
                                                                                let arg = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                                                                                match invoke_checked("cleanup_run_workspace", arg).await {
                                                                                    Ok(_) => show_toast(&t(locale.get_untracked(), "runs.cleanup_done")),
                                                                                    Err(error) => show_toast(&localize_backend(
                                                                                        locale.get_untracked(),
                                                                                        &js_error_text(error),
                                                                                    )),
                                                                                }
                                                                                refresh_runs(runs, locale);
                                                                            });
                                                                        }>{compose_icon("trash")}</button>
                                                                }
                                                            })}
                                                            {cleaned.then(|| view! {
                                                                <span class="run-cleaned" data-testid="run-cleaned">
                                                                    {t(locale.get(), "runs.cleaned")}
                                                                </span>
                                                            })}
                                                        </div>
                                                    </div>
                                                    {cleanup_error.filter(|error| !error.trim().is_empty()).map(|error| view! {
                                                        <div class="context-error">{error}</div>
                                                    })}
                                                    {progress.map(|progress| run_progress_meter(progress, locale.get()))}
                                                    {method_progress.map(|progress| view! {
                                                        <div class="method-search-card-progress">
                                                            <span>{progress.phase}</span>
                                                            <span>{format!("{} / {}", progress.candidate_count, progress.successful_count)}</span>
                                                            <strong>{progress.best_primary.map(|value| format!("{value:.6}")).unwrap_or_else(|| "—".into())}</strong>
                                                        </div>
                                                    })}
                                                    {method_search.then(|| view! {
                                                        <MethodSearchRunPanel
                                                            run_id=method_run_id
                                                            locale=locale
                                                            modal=modal
                                                            modal_artifact=modal_artifact
                                                        />
                                                    })}
                                                    {remote_workdir.map(|workdir| view! {
                                                        <div class="run-remote">
                                                            <span>{t(locale.get(), "runs.remote_workdir")}</span>
                                                            <code>{workdir}</code>
                                                        </div>
                                                    })}
                                                    {poll_error.filter(|error| !error.trim().is_empty()).map(|error| view! {
                                                        <div class="context-error">{error}</div>
                                                    })}
                                                    <RunDetailDisclosure run_id=detail_run_id locale=locale />
                                                    {(!produced.is_empty()).then(|| view! {
                                                        <div class="run-artifacts">
                                                            <span class="run-artifacts-label">{t(locale.get(), "runs.artifacts")}</span>
                                                            {produced.into_iter().map(|(artifact_id, name)| {
                                                                // `artifact:<id>` is the preview spelling that reads the
                                                                // registered row, so a harvested output opens without
                                                                // needing its path to still resolve under the workspace.
                                                                let kind = file_kind(&name).unwrap_or("text").to_string();
                                                                let target = (format!("artifact:{artifact_id}"), name.clone(), kind);
                                                                view! {
                                                                    <button type="button" class="run-artifact"
                                                                        on:click=move |_| {
                                                                            // The viewer renders below this overlay, so leaving
                                                                            // the Runs modal open would just cover the file.
                                                                            modal.set(None);
                                                                            modal_artifact.set(Some(target.clone()));
                                                                        }>
                                                                        {name}
                                                                    </button>
                                                                }
                                                            }).collect_view()}
                                                        </div>
                                                    })}
                                                </div>
                                            }
                                                }).collect_view()
                                            }}
                                        </section>
                                    }
                                }}
                            }.into_view()
                        }
                    }}
                </div>
            </div>
        }.into_view()
    }
}
