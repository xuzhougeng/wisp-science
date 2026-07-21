use crate::{
    bindings::{invoke, listen, sync_pet},
    dto::{AgentEvent, PetStatus},
};
use leptos::*;
use serde::Deserialize;
use std::collections::HashSet;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};

#[derive(Clone, Default, PartialEq, Eq)]
struct DesktopPetActivity {
    running: HashSet<String>,
    waiting: HashSet<String>,
    waiting_target: Option<String>,
    reviewing: HashSet<String>,
    transient: String,
    sequence: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PetRuntimeSnapshot {
    #[serde(default)]
    running: Vec<String>,
    #[serde(default)]
    waiting: Vec<String>,
    #[serde(default)]
    reviewing: Vec<String>,
}

impl DesktopPetActivity {
    fn replace_from_snapshot(&mut self, snapshot: PetRuntimeSnapshot) {
        self.running = snapshot.running.into_iter().collect();
        self.waiting = snapshot.waiting.into_iter().collect();
        self.retarget_waiting();
        self.reviewing = snapshot.reviewing.into_iter().collect();
        self.transient.clear();
    }

    fn retarget_waiting(&mut self) {
        if self
            .waiting_target
            .as_ref()
            .is_none_or(|frame_id| !self.waiting.contains(frame_id))
        {
            self.waiting_target = self.waiting.iter().next().cloned();
        }
    }

    fn mark_running(&mut self, frame_id: String) {
        self.waiting.remove(&frame_id);
        self.retarget_waiting();
        self.reviewing.remove(&frame_id);
        self.running.insert(frame_id);
        self.transient.clear();
    }

    fn mark_waiting(&mut self, frame_id: String) {
        self.running.insert(frame_id.clone());
        self.waiting.insert(frame_id.clone());
        self.waiting_target = Some(frame_id);
        self.transient.clear();
    }

    fn finish(&mut self, frame_id: &str, transient: &str) {
        self.running.remove(frame_id);
        self.waiting.remove(frame_id);
        self.retarget_waiting();
        self.reviewing.remove(frame_id);
        self.transient = transient.to_string();
        self.sequence = self.sequence.wrapping_add(1);
    }

    fn apply_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::User { frame_id, .. }
            | AgentEvent::Text { frame_id, .. }
            | AgentEvent::Reasoning { frame_id, .. }
            | AgentEvent::ToolCall { frame_id, .. }
            | AgentEvent::Stdout { frame_id, .. }
            | AgentEvent::CorrectionStarted { frame_id, .. } => self.mark_running(frame_id),
            AgentEvent::ToolResult {
                frame_id,
                ok: false,
                ..
            }
            | AgentEvent::Error { frame_id, .. } => self.finish(&frame_id, "failed"),
            AgentEvent::ReviewFailed { frame_id, .. } => self.finish(&frame_id, "failed"),
            AgentEvent::ToolResult {
                frame_id, ok: true, ..
            } => self.mark_running(frame_id),
            AgentEvent::ReviewStarted { frame_id } | AgentEvent::Review { frame_id, .. } => {
                self.waiting.remove(&frame_id);
                self.retarget_waiting();
                self.running.insert(frame_id.clone());
                self.reviewing.insert(frame_id);
                self.transient.clear();
            }
            AgentEvent::Done { frame_id, .. } => self.finish(&frame_id, "jumping"),
            AgentEvent::DelegationCompleted {
                frame_id, status, ..
            } => self.finish(
                &frame_id,
                if status == "succeeded" {
                    "jumping"
                } else {
                    "failed"
                },
            ),
            AgentEvent::MessageBoundary { .. }
            | AgentEvent::Usage { .. }
            | AgentEvent::Compaction { .. }
            | AgentEvent::ContextWarning { .. }
            | AgentEvent::Diff { .. }
            | AgentEvent::FileChanged { .. }
            | AgentEvent::Resources { .. } => {}
        }
    }

    fn state(&self) -> &str {
        if !self.waiting.is_empty() {
            "waiting"
        } else if self.transient == "failed" {
            "failed"
        } else if !self.reviewing.is_empty() {
            "review"
        } else if !self.running.is_empty() {
            "running"
        } else if self.transient == "jumping" {
            "jumping"
        } else {
            "idle"
        }
    }
}

fn install_listener(event: &'static str, callback: Closure<dyn FnMut(JsValue)>) {
    let function = callback
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    callback.forget();
    spawn_local(async move {
        let _ = listen(event, &function).await;
    });
}

fn refresh_desktop_pet(status: RwSignal<PetStatus>, activity: RwSignal<DesktopPetActivity>) {
    spawn_local(async move {
        let value = invoke("get_pet", JsValue::UNDEFINED).await;
        let visible = serde_wasm_bindgen::from_value::<PetStatus>(value)
            .map(|next| {
                let visible = next.enabled && next.asset.is_some();
                status.set(next);
                visible
            })
            .unwrap_or(false);
        let args = serde_wasm_bindgen::to_value(&serde_json::json!({ "visible": visible }))
            .unwrap_or(JsValue::UNDEFINED);
        let _ = invoke("set_pet_window_visible", args).await;

        let snapshot = invoke("get_pet_runtime_status", JsValue::UNDEFINED).await;
        if let Ok(snapshot) = serde_wasm_bindgen::from_value::<PetRuntimeSnapshot>(snapshot) {
            activity.update(|current| current.replace_from_snapshot(snapshot));
        }
    });
}

fn desktop_state_label(state: &str) -> &'static str {
    match state {
        "running" => "Working",
        "review" => "Reviewing",
        "waiting" => "Needs you",
        "failed" => "Failed",
        "jumping" => "Done",
        _ => "Idle",
    }
}

#[component]
pub(crate) fn PetDesktop() -> impl IntoView {
    let status = create_rw_signal(PetStatus::default());
    let activity = create_rw_signal(DesktopPetActivity::default());

    if let Some(root) = document().document_element() {
        let _ = root.set_attribute("class", "pet-window-shell");
    }
    if let Some(body) = document().body() {
        body.set_class_name("pet-window-shell");
    }

    refresh_desktop_pet(status, activity);
    install_listener(
        "pet-config-changed",
        Closure::wrap(Box::new(move |_: JsValue| {
            refresh_desktop_pet(status, activity);
        }) as Box<dyn FnMut(JsValue)>),
    );
    install_listener(
        "agent",
        Closure::wrap(Box::new(move |payload: JsValue| {
            if let Ok(event) = serde_wasm_bindgen::from_value::<AgentEvent>(payload) {
                activity.update(|current| current.apply_agent_event(event));
            }
        }) as Box<dyn FnMut(JsValue)>),
    );
    for event_name in ["confirm-request", "permission-request"] {
        install_listener(
            event_name,
            Closure::wrap(Box::new(move |payload: JsValue| {
                let Ok(value) = serde_wasm_bindgen::from_value::<serde_json::Value>(payload) else {
                    return;
                };
                let frame_id = value
                    .get("frame_id")
                    .or_else(|| value.get("frameId"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if !frame_id.is_empty() {
                    activity.update(|current| current.mark_waiting(frame_id.to_string()));
                }
            }) as Box<dyn FnMut(JsValue)>),
        );
    }

    create_effect(move |_| {
        let status = status.get();
        let activity = activity.get();
        let state = activity.state();
        let visible = status.enabled && status.asset.is_some();
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
            "sequence": activity.sequence,
            "frameCounts": frame_counts,
            "desktop": true,
            "roam": false,
        });
        if let Ok(json) = serde_json::to_string(&config) {
            sync_pet("wisp-pet", &json);
        }
    });

    view! {
        <main class="pet-window-root" data-testid="pet-window-root">
            <button id="wisp-pet" class="wisp-pet desktop-pet" type="button"
                data-testid="wisp-pet"
                data-tauri-drag-region="deep"
                on:click:undelegated=move |_| {
                    let Some(session_id) = activity.with_untracked(|current| current.waiting_target.clone()) else {
                        return;
                    };
                    spawn_local(async move {
                        let args = serde_wasm_bindgen::to_value(
                            &serde_json::json!({ "sessionId": session_id }),
                        )
                        .unwrap_or(JsValue::UNDEFINED);
                        let _ = invoke("open_pet_session", args).await;
                    });
                }
                aria-label=move || {
                    let current = activity.get();
                    let name = status.get().asset.map(|asset| asset.display_name)
                        .unwrap_or_else(|| "Pet".into());
                    format!("{name}: {}", desktop_state_label(current.state()))
                }>
                <span class="wisp-pet-sprite" aria-hidden="true"></span>
                <span class="wisp-pet-status" aria-hidden="true"></span>
                <span class="wisp-pet-state-label">
                    {move || desktop_state_label(activity.get().state())}
                </span>
            </button>
        </main>
    }
}

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
        let is_running = active.as_ref().is_some_and(|id| running.get().contains(id));
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
