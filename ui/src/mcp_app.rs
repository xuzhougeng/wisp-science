use crate::app_support::{compose_icon, js_error_text};
use crate::bindings::{
    import_motif_dna_file, mount_mcp_app, park_mcp_app, request_motif_selection,
};
use crate::i18n::{t, use_locale};
use crate::text::unique_dom_id;
use leptos::*;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MotifSelection {
    pub(crate) record_name: String,
    pub(crate) record_id: String,
    pub(crate) feature_name: Option<String>,
    pub(crate) molecule: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) strand: String,
    pub(crate) sequence: String,
}

impl MotifSelection {
    pub(crate) fn length_bp(&self) -> usize {
        self.sequence.chars().count()
    }

    pub(crate) fn composer_text(&self) -> String {
        let feature = self
            .feature_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map(|name| format!("Feature: {name}\n"))
            .unwrap_or_default();
        format!(
            "[Motif sequence selection]\nRecord: {}{}\n{}Coordinates: {}-{} ({})\nLength: {} bp\nMolecule: {}\nSequence: {}",
            self.record_name,
            if self.record_id.is_empty() { String::new() } else { format!(" [{}]", self.record_id) },
            feature,
            self.start,
            self.end,
            self.strand,
            self.length_bp(),
            self.molecule,
            self.sequence,
        )
    }
}

fn supports_host_dna_import(payload: &serde_json::Value) -> bool {
    payload
        .pointer("/tool/name")
        .and_then(serde_json::Value::as_str)
        == Some("motif_open_workbench")
}

pub(crate) fn active_motif_instance(
    apps: &std::collections::HashMap<String, String>,
) -> Option<String> {
    apps.iter().find_map(|(instance_id, payload_json)| {
        serde_json::from_str::<serde_json::Value>(payload_json)
            .ok()
            .filter(supports_host_dna_import)
            .map(|_| instance_id.clone())
    })
}

pub(crate) fn mcp_app_title(payload: &serde_json::Value) -> String {
    payload
        .pointer("/tool/title")
        .or_else(|| payload.pointer("/tool/annotations/title"))
        .or_else(|| payload.pointer("/tool/name"))
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("MCP App")
        .to_string()
}

pub(crate) fn mcp_app_identity(payload: &serde_json::Value) -> &str {
    let raw = payload
        .pointer("/resource/uri")
        .or_else(|| payload.pointer("/tool/name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("app");
    raw.split_once(['?', '#'])
        .map(|(base, _)| base)
        .filter(|base| !base.is_empty())
        .unwrap_or(raw)
}

pub(crate) fn mcp_app_instance_id(frame_id: &str, payload: &serde_json::Value) -> String {
    format!("mcp-app:{frame_id}:{}", mcp_app_identity(payload))
}

#[component]
pub(crate) fn McpAppPreview(
    instance_id: String,
    payload_json: String,
    on_selection: Callback<MotifSelection>,
) -> impl IntoView {
    let locale = use_locale();
    let can_import = serde_json::from_str::<serde_json::Value>(&payload_json)
        .is_ok_and(|payload| supports_host_dna_import(&payload));
    let dom_id = unique_dom_id("center-mcp-app");
    let import_busy = create_rw_signal(false);
    let import_status = create_rw_signal(None::<(bool, String)>);
    {
        let mount_id = instance_id.clone();
        let mount_dom_id = dom_id.clone();
        let mount_payload = payload_json.clone();
        create_effect(move |_| {
            let _ = mount_mcp_app(&mount_id, &mount_dom_id, &mount_payload);
        });
    }
    {
        let parked_id = instance_id.clone();
        on_cleanup(move || park_mcp_app(&parked_id));
    }
    let import_id = instance_id.clone();
    let selection_id = instance_id.clone();
    view! {
        <div class="center-mcp-app-shell" class:has-toolbar=can_import>
            {can_import.then(|| view! {
                <div class="center-mcp-app-toolbar">
                    <button type="button" class="center-mcp-import" disabled=move || import_busy.get()
                        on:click=move |_| {
                            let id = import_id.clone();
                            import_busy.set(true);
                            import_status.set(None);
                            spawn_local(async move {
                                match import_motif_dna_file(&id).await {
                                    Ok(value) => {
                                        let imported = js_sys::Reflect::get(&value, &"imported".into())
                                            .ok().and_then(|v| v.as_bool()).unwrap_or(false);
                                        if imported {
                                            let filename = js_sys::Reflect::get(&value, &"filename".into())
                                                .ok().and_then(|v| v.as_string()).unwrap_or_default();
                                            import_status.set(Some((true, filename)));
                                        }
                                    }
                                    Err(error) => import_status.set(Some((false, js_error_text(error)))),
                                }
                                import_busy.set(false);
                            });
                        }>
                        {compose_icon("upload")}
                        <span>{move || t(locale.get(), if import_busy.get() { "motif.importing" } else { "motif.import_local" })}</span>
                    </button>
                    <button type="button" class="center-mcp-import"
                        on:click=move |_| {
                            let id = selection_id.clone();
                            import_status.set(None);
                            spawn_local(async move {
                                match request_motif_selection(&id).await {
                                    Ok(value) => match serde_wasm_bindgen::from_value::<MotifSelection>(value) {
                                        Ok(selection) => on_selection.call(selection),
                                        Err(error) => import_status.set(Some((false, error.to_string()))),
                                    },
                                    Err(error) => import_status.set(Some((false, js_error_text(error)))),
                                }
                            });
                        }>
                        {compose_icon("chat")}
                        <span>{move || t(locale.get(), "motif.selection_to_chat")}</span>
                    </button>
                    <span class="center-mcp-import-hint">{move || t(locale.get(), "motif.import_hint")}</span>
                    {move || import_status.get().map(|(ok, text)| view! {
                        <span class="center-mcp-import-status" class:fail=!ok>
                            {if ok { format!("{}: {text}", t(locale.get(), "motif.imported")) } else { text }}
                        </span>
                    })}
                </div>
            })}
            <div class="center-mcp-app" id=dom_id data-mcp-app-id=instance_id></div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_dna_import_is_only_exposed_for_motif_open_workbench() {
        assert!(supports_host_dna_import(&serde_json::json!({
            "tool": { "name": "motif_open_workbench" }
        })));
        assert!(!supports_host_dna_import(&serde_json::json!({
            "tool": { "name": "motif_create_workbench_artifact" }
        })));
        assert!(!supports_host_dna_import(&serde_json::json!({})));
    }

    #[test]
    fn mcp_app_instance_id_reuses_resource_uri_not_presentation() {
        let open = serde_json::json!({
            "tool": { "name": "figure_open", "title": "Open Scientific Figure Library" },
            "resource": { "uri": "ui://figure/library.html" },
        });
        let search = serde_json::json!({
            "tool": { "name": "figure_search", "title": "Search scientific figure templates" },
            "resource": { "uri": "ui://figure/library.html?q=survival#hits" },
        });
        assert_eq!(
            mcp_app_instance_id("sess-1", &open),
            "mcp-app:sess-1:ui://figure/library.html"
        );
        assert_eq!(
            mcp_app_instance_id("sess-1", &search),
            mcp_app_instance_id("sess-1", &open)
        );
        assert_ne!(
            mcp_app_instance_id("sess-a", &open),
            mcp_app_instance_id("sess-b", &open)
        );
    }

    #[test]
    fn mcp_app_instance_id_falls_back_to_tool_name() {
        assert_eq!(
            mcp_app_instance_id(
                "sess-1",
                &serde_json::json!({ "tool": { "name": "open_app" } })
            ),
            "mcp-app:sess-1:open_app"
        );
        assert_eq!(
            mcp_app_instance_id("sess-1", &serde_json::json!({})),
            "mcp-app:sess-1:app"
        );
    }

    #[test]
    fn motif_selection_formats_bounded_structured_composer_context() {
        let selection = MotifSelection {
            record_name: "pET-28a".into(),
            record_id: "vector-1".into(),
            feature_name: Some("MBP".into()),
            molecule: "dna".into(),
            start: 10,
            end: 17,
            strand: "forward".into(),
            sequence: "ACGTACGT".into(),
        };
        let text = selection.composer_text();
        assert!(text.contains("pET-28a [vector-1]"));
        assert!(text.contains("Feature: MBP"));
        assert!(text.contains("Coordinates: 10-17 (forward)"));
        assert!(text.contains("Length: 8 bp"));
        assert!(text.ends_with("Sequence: ACGTACGT"));
    }
}
