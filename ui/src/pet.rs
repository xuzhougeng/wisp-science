use crate::{bindings::sync_pet, dto::PetStatus};
use leptos::*;
use std::collections::HashSet;

#[component]
pub(crate) fn PetOverlay(
    status: RwSignal<PetStatus>,
    active_session: RwSignal<Option<String>>,
    running: RwSignal<HashSet<String>>,
    approval_pending: RwSignal<HashSet<String>>,
    activity: RwSignal<(String, u64)>,
    show_projects: RwSignal<bool>,
    show_settings: RwSignal<bool>,
    center_file_open: Memo<bool>,
) -> impl IntoView {
    create_effect(move |_| {
        let status = status.get();
        let active = active_session.get();
        let is_running = active
            .as_ref()
            .is_some_and(|id| running.get().contains(id));
        let needs_user = active
            .as_ref()
            .is_some_and(|id| approval_pending.get().contains(id));
        let (activity_state, sequence) = activity.get();
        let state = if needs_user {
            "waiting"
        } else if activity_state == "failed" {
            "failed"
        } else if is_running {
            if activity_state == "review" {
                "review"
            } else {
                "running"
            }
        } else if matches!(activity_state.as_str(), "jumping" | "waving") {
            activity_state.as_str()
        } else {
            "idle"
        };
        let visible = status.enabled
            && status.asset.is_some()
            && !show_projects.get()
            && !show_settings.get()
            && !center_file_open.get();
        let (src, name, frame_counts) = status
            .asset
            .as_ref()
            .map(|asset| {
                (
                    asset.spritesheet_data_url.clone(),
                    asset.display_name.clone(),
                    asset.frame_counts.clone(),
                )
            })
            .unwrap_or_default();
        let config = serde_json::json!({
            "visible": visible,
            "src": src,
            "name": name,
            "state": state,
            "sequence": sequence,
            "frameCounts": frame_counts,
        });
        if let Ok(json) = serde_json::to_string(&config) {
            sync_pet("wisp-pet", &json);
        }
    });

    view! {
        <button id="wisp-pet" class="wisp-pet" type="button" data-testid="wisp-pet"
            aria-label=move || status.get().asset.map(|asset| asset.display_name).unwrap_or_else(|| "Pet".into())>
            <span class="wisp-pet-sprite" aria-hidden="true"></span>
            <span class="wisp-pet-status" aria-hidden="true"></span>
        </button>
    }
}
