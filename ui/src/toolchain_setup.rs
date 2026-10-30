//! Install uv, Python, R, and Node into Wisp's own toolchain from the local
//! environment panel (onboarding, Settings → General, Capabilities). The
//! backend keeps running after the panel closes; reopening it shows progress.

use crate::app_support::{js_error_text, show_toast};
use crate::bindings::invoke_checked;
use crate::dto::{BootstrapStatus, ToolchainStatus};
use crate::i18n::{localize_backend, t, tf, Locale};
use crate::text::event_target_checked;
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::collections::BTreeSet;
use wasm_bindgen::JsValue;

/// Tool id, display name, i18n key of its note, and the Local path that says
/// whether it is already available.
const TOOLS: [(&str, &str, &str, &str); 4] = [
    ("uv", "uv", "toolchain.uv", "uv_executable"),
    (
        "python",
        "Python 3.12",
        "toolchain.python",
        "python_executable",
    ),
    ("r", "R", "toolchain.r", "rscript_executable"),
    ("node", "Node.js", "toolchain.node", "node_executable"),
];

fn step_text(locale: Locale, status: &ToolchainStatus) -> String {
    let (key, tool) = match status.label.split_once(':') {
        Some(("install", tool)) => ("toolchain.step.install", tool),
        _ => (
            match status.label.as_str() {
                "venv" => "toolchain.step.venv",
                "packages" => "toolchain.step.packages",
                "save" => "toolchain.step.save",
                _ => "toolchain.step.pixi",
            },
            "",
        ),
    };
    let name = TOOLS
        .iter()
        .find(|(id, ..)| *id == tool)
        .map(|(_, name, ..)| *name)
        .unwrap_or(tool);
    tf(
        locale,
        "toolchain.progress",
        &[
            ("step", &status.step.max(1).to_string()),
            ("total", &status.total.max(1).to_string()),
            ("action", &tf(locale, key, &[("tool", name)])),
        ],
    )
}

#[component]
pub(crate) fn ToolchainSetup(
    locale: RwSignal<Locale>,
    bootstrap: RwSignal<Option<BootstrapStatus>>,
) -> impl IntoView {
    let status = create_rw_signal(None::<ToolchainStatus>);
    let selected = create_rw_signal(BTreeSet::<&'static str>::new());
    // Defaults follow detection until the user picks tools themselves.
    let touched = create_rw_signal(false);
    let error = create_rw_signal(None::<String>);
    // Memos, so progress events that do not change these answers leave the
    // checkboxes and buttons in place instead of rebuilding them mid-click.
    let running =
        create_memo(move |_| status.with(|s| s.as_ref().is_some_and(|s| s.state == "running")));
    // Tools neither detected nor installed by Wisp, or `None` while the
    // background check has not reported. Detected tools are used as they are.
    let missing = create_memo(move |_| {
        let installed =
            status.with(|s| s.as_ref().map(|s| s.installed.clone()).unwrap_or_default());
        bootstrap.with(|b| {
            b.as_ref()
                .and_then(|b| b.local_environment.as_ref())
                .map(|report| {
                    TOOLS
                        .iter()
                        .filter(|(id, _, _, key)| {
                            !report.paths.contains_key(*key)
                                && !installed.iter().any(|tool| tool == id)
                        })
                        .map(|(id, ..)| *id)
                        .collect::<Vec<_>>()
                })
        })
    });
    let managed = move |id: &str| {
        status.with(|s| {
            s.as_ref()
                .is_some_and(|s| s.installed.iter().any(|tool| tool == id))
        })
    };
    let offering = create_memo(move |_| {
        running.get() || missing.with(|tools| tools.as_ref().is_some_and(|tools| !tools.is_empty()))
    });

    spawn_local(async move {
        if let Ok(value) = invoke_checked("get_toolchain_status", JsValue::UNDEFINED).await {
            if let Ok(value) = serde_wasm_bindgen::from_value::<ToolchainStatus>(value) {
                status.set(Some(value));
            }
        }
    });
    crate::session_timer::listen(
        "toolchain-install",
        Callback::new(move |next: ToolchainStatus| {
            let finished = status.with_untracked(|previous| {
                previous.as_ref().is_some_and(|p| p.state == "running") && next.state == "done"
            });
            if next.state == "done" {
                // What just installed no longer needs installing.
                selected
                    .update(|set| set.retain(|id| !next.installed.iter().any(|tool| tool == id)));
            }
            status.set(Some(next));
            if finished {
                show_toast(&t(locale.get_untracked(), "toolchain.done"));
            }
        }),
    );
    create_effect(move |_| {
        if touched.get() {
            return;
        }
        // Suggest every missing tool; before detection reports, none.
        selected.set(missing.get().unwrap_or_default().into_iter().collect());
    });

    let install = move |_| {
        let offered = missing.get_untracked().unwrap_or_default();
        let tools = selected
            .get_untracked()
            .into_iter()
            .filter(|id| offered.contains(id))
            .collect::<Vec<_>>();
        error.set(None);
        spawn_local(async move {
            let args = to_value(&serde_json::json!({ "tools": tools })).unwrap();
            match invoke_checked("install_toolchain", args).await {
                Ok(value) => {
                    if let Ok(value) = serde_wasm_bindgen::from_value::<ToolchainStatus>(value) {
                        // A progress event may already be newer than this reply.
                        status.update(|current| {
                            if current.as_ref().map_or(true, |c| c.state != "running") {
                                *current = Some(value);
                            }
                        });
                    }
                }
                Err(value) => error.set(Some(localize_backend(
                    locale.get_untracked(),
                    &js_error_text(value),
                ))),
            }
        });
    };
    let cancel = move |_| {
        spawn_local(async move {
            let _ = invoke_checked("cancel_toolchain_install", JsValue::UNDEFINED).await;
        });
    };

    view! {
        <div class="toolchain-setup" data-testid="toolchain-setup">
            <h4>{move || t(locale.get(), "toolchain.title")}</h4>
            <p class="hint">{move || {
                if missing.with(|tools| tools.as_ref().is_some_and(Vec::is_empty)) && !running.get() {
                    return t(locale.get(), "toolchain.all_found");
                }
                let root = status.with(|s| s.as_ref().map(|s| s.root.clone()).unwrap_or_default());
                tf(locale.get(), "toolchain.hint", &[("root", &root)])
            }}</p>
            <div class="toolchain-tools">
                {TOOLS.into_iter().map(|(id, name, note, _)| {
                    let offered = move || missing.with(|tools| tools.as_ref().is_some_and(|tools| tools.contains(&id)));
                    view! {
                        <label class="toolchain-tool">
                            {move || offered().then(|| view! {
                                <input type="checkbox"
                                    disabled=move || running.get()
                                    prop:checked=move || selected.with(|set| set.contains(id))
                                    on:change=move |ev| {
                                        touched.set(true);
                                        let on = event_target_checked(&ev);
                                        selected.update(|set| {
                                            if on { set.insert(id); } else { set.remove(id); }
                                        });
                                    } />
                            })}
                            <span class="toolchain-name">{name}</span>
                            <span class="toolchain-note">{move || t(locale.get(), note)}</span>
                            <span class="toolchain-state">{move || t(locale.get(), if managed(id) {
                                "toolchain.managed"
                            } else if missing.with(Option::is_none) {
                                "toolchain.checking"
                            } else if offered() {
                                "toolchain.absent"
                            } else {
                                "toolchain.found"
                            })}</span>
                        </label>
                    }
                }).collect_view()}
            </div>
            {move || status.get().filter(|s| s.state != "idle").map(|s| {
                let loc = locale.get();
                let summary = match s.state.as_str() {
                    "running" => step_text(loc, &s),
                    "done" => t(loc, "toolchain.done"),
                    "cancelled" => t(loc, "toolchain.cancelled"),
                    _ => s.error.as_deref().map(|error| localize_backend(loc, error)).unwrap_or_else(|| t(loc, "toolchain.failed")),
                };
                let class = match s.state.as_str() {
                    "done" => "settings-status ok",
                    "failed" => "settings-status fail",
                    _ => "hint",
                };
                let tail = s.log.iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join("\n");
                view! {
                    <p class=class role=if s.state == "failed" { "alert" } else { "status" }>{summary}</p>
                    {(!tail.is_empty()).then(|| view! { <pre class="toolchain-log">{tail}</pre> })}
                }
            })}
            {move || error.get().map(|message| view! { <p class="settings-status fail" role="alert">{message}</p> })}
            {move || offering.get().then(|| view! {
                <div class="environment-actions">
                    {move || if running.get() {
                        view! {
                            <button type="button" on:click=cancel>{move || t(locale.get(), "toolchain.cancel")}</button>
                        }.into_view()
                    } else {
                        view! {
                            <button type="button" class="primary"
                                disabled=move || selected.with(BTreeSet::is_empty)
                                on:click=install>{move || {
                                    let retry = status.with(|s| s.as_ref().is_some_and(|s| s.state == "failed" || s.state == "cancelled"));
                                    t(locale.get(), if retry { "toolchain.retry" } else { "toolchain.install" })
                                }}</button>
                        }.into_view()
                    }}
                </div>
                <p class="hint toolchain-later">{move || t(locale.get(), "toolchain.later")}</p>
            })}
        </div>
    }
}
