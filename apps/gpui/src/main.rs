#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod acp;
mod agents;
mod aside;
mod assets;
mod assistant;
mod backend;
mod browser;
mod calendar;
mod channels;
mod cif;
mod codex_pool;
mod compose;
mod console;
mod controls;
mod decision;
mod demos;
mod explore;
mod favorites;
mod files;
mod find;
mod generated;
mod home;
mod i18n;
mod i18n_zh;
mod inline;
mod journey;
mod layers;
mod links;
mod mandates;
mod memory;
mod memos;
mod method_search;
mod model_view;
mod molecules;
mod notebook;
mod notify;
mod onboarding;
mod organise;
mod outline;
mod palette;
mod panel;
mod papers;
mod pdf_pages;
mod pet;
mod plugins;
mod present;
mod project_windows;
mod queue;
mod remote;
mod review;
mod run_review;
mod schedules;
mod sequences;
mod settings;
mod share;
mod slash;
mod smiles;
mod state;
mod sync;
mod tabs;
mod terminal;
mod terminal_screen;
mod theme;
mod toolchain;
mod trajectory;
mod transfers;
mod usage;
mod webview_import;
mod workflows;
mod workspace;

use gpui::{prelude::*, *};
use gpui_component::Root;
use std::sync::Arc;

actions!(wisp, [Quit, NewWindow]);

/// How far a new window is moved from the centre, so it does not cover the
/// ones that are open: a step down and right for each, starting over after
/// eight.
pub fn cascade(open: usize) -> f32 {
    (open % 8) as f32 * 28.
}

/// Opens a window on the same host: its own page, conversation and panels,
/// the same projects and settings. Returns whether it opened.
pub fn open_window(backend: backend::Backend, cx: &mut App) -> bool {
    open_window_on(backend, None, cx)
}

/// Opens a window as [`open_window`] does, in `project` when one is given
/// rather than on the home page.
pub fn open_window_on(backend: backend::Backend, project: Option<String>, cx: &mut App) -> bool {
    open_window_noting(backend, project, true, cx)
}

/// Opens a window; one for a project is added to the saved list when
/// `note` is set, and taken off it when it is closed.
fn open_window_noting(
    backend: backend::Backend,
    project: Option<String>,
    note: bool,
    cx: &mut App,
) -> bool {
    let shift = cascade(cx.windows().len());
    let mut bounds = Bounds::centered(None, size(px(1180.), px(780.)), cx);
    bounds.origin.x += px(shift);
    bounds.origin.y += px(shift);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(820.), px(540.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Wisp Science".into()),
                ..Default::default()
            }),
            app_id: Some("science.wisp.gpui".into()),
            ..Default::default()
        },
        |window, cx| {
            let keeper = backend.clone();
            let workspace = cx.new(|cx| workspace::Workspace::new(backend, window, cx));
            if let Some(project) = project {
                workspace.update(cx, |workspace, cx| {
                    workspace.enter_project(project.clone(), window, cx)
                });
                // Kept so it opens again at the next start, until it is
                // closed; quitting keeps it.
                if note {
                    save_project_windows(&keeper, project_windows::note(&project, true), cx);
                }
                window.on_window_should_close(cx, move |_, cx| {
                    save_project_windows(&keeper, project_windows::note(&project, false), cx);
                    true
                });
            }
            cx.new(|cx| Root::new(workspace, window, cx))
        },
    );
    if let Err(error) = &opened {
        eprintln!("Cannot open GPUI window: {error:#}");
    }
    opened.is_ok()
}

/// Saves the list of project windows. Nothing is said if it fails: the
/// windows simply do not open again at the next start.
fn save_project_windows(backend: &backend::Backend, list: String, cx: &App) {
    let reply = backend.set_preference(project_windows::PREFERENCE, list);
    cx.background_spawn(async move {
        let _ = backend::receive(reply).await;
    })
    .detach();
}

/// Opens again the project windows that were open when Wisp last quit,
/// for the projects that are still there.
fn reopen_project_windows(backend: backend::Backend, cx: &mut App) {
    let saved = backend.preference(project_windows::PREFERENCE);
    let present = backend.request::<Vec<wisp_dto::ProjectSummary>>(
        "list_projects",
        None,
        serde_json::json!({}),
    );
    cx.spawn(async move |cx| {
        let Ok(Some(saved)) = backend::receive(saved).await else {
            return;
        };
        let Ok(present) = backend::receive(present).await else {
            return;
        };
        let projects = project_windows::decode(&saved)
            .into_iter()
            .filter(|project| present.iter().any(|known| &known.id == project))
            .collect::<Vec<_>>();
        let _ = cx.update(|cx| {
            // Saved once for all of them; a project that went away is not
            // kept.
            save_project_windows(&backend, project_windows::replace(&projects), cx);
            for project in projects {
                open_window_noting(backend.clone(), Some(project), false, cx);
            }
        });
    })
    .detach();
}

fn main() -> anyhow::Result<()> {
    // Started by an ACP agent as its MCP server: carry its stdio to the
    // host's bridge and nothing else; no window opens.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(wisp_server::acp_bridge::RELAY_FLAG) {
        let code = wisp_server::acp_bridge::relay_main(args.get(2).map_or("", String::as_str));
        std::process::exit(if code == std::process::ExitCode::SUCCESS {
            0
        } else {
            1
        });
    }
    // Agents that only start programs reach Wisp's tools through this
    // same executable in relay mode.
    if let Ok(program) = std::env::current_exe() {
        wisp_server::acp_bridge::set_relay(program);
    }
    let runtime = Arc::new(tokio::runtime::Runtime::new()?);
    let backend = backend::Backend::new(runtime.clone());
    let ui_backend = backend.clone();
    let quit_backend = backend.clone();
    let quit_runtime = runtime.clone();
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            theme::install(cx);
            // The system's language until the saved choice has been read.
            i18n::apply(i18n::Language::System);
            cx.on_app_quit(move |_| {
                // GPUI gives returned futures only 100ms and macOS terminates the
                // process without returning from Application::run. Finish the host's
                // bounded shutdown here, before allowing native termination.
                quit_runtime.block_on(quit_backend.shutdown());
                async {}
            })
            .detach();
            cx.on_action(|_: &Quit, cx| cx.quit());
            let window_backend = ui_backend.clone();
            cx.on_action(move |_: &NewWindow, cx| {
                open_window(window_backend.clone(), cx);
            });
            let primary = if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            };
            cx.bind_keys([
                KeyBinding::new(&format!("{primary}-q"), Quit, None),
                KeyBinding::new(&format!("{primary}-shift-n"), NewWindow, None),
            ]);
            // After the input component's own keys, so the composer's win.
            cx.bind_keys(workspace::composer_bindings());
            cx.set_menus(vec![Menu {
                name: "Wisp Science".into(),
                items: vec![
                    MenuItem::action("New Window", NewWindow),
                    MenuItem::separator(),
                    MenuItem::action("Quit", Quit),
                ],
            }]);
            if !open_window(ui_backend.clone(), cx) {
                cx.quit();
            }
            reopen_project_windows(ui_backend, cx);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    // Cancel turns, deny pending approvals and reclaim managed runtimes before exit.
    runtime.block_on(backend.shutdown());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::cascade;

    #[test]
    fn new_windows_step_away_from_the_ones_that_are_open() {
        assert_eq!(cascade(0), 0.);
        assert_eq!(cascade(1), 28.);
        assert_eq!(cascade(7), 196.);
        // After eight the steps start over instead of leaving the screen.
        assert_eq!(cascade(8), 0.);
        assert_eq!(cascade(9), 28.);
    }
}
