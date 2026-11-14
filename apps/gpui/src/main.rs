#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod aside;
mod assets;
mod backend;
mod calendar;
mod compose;
mod controls;
mod favorites;
mod files;
mod find;
mod home;
mod i18n;
mod i18n_zh;
mod inline;
mod journey;
mod layers;
mod links;
mod memory;
mod memos;
mod model_view;
mod notebook;
mod notify;
mod organise;
mod outline;
mod palette;
mod panel;
mod present;
mod queue;
mod review;
mod settings;
mod state;
mod theme;
mod trajectory;
mod usage;
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
            let workspace = cx.new(|cx| workspace::Workspace::new(backend, window, cx));
            cx.new(|cx| Root::new(workspace, window, cx))
        },
    );
    if let Err(error) = &opened {
        eprintln!("Cannot open GPUI window: {error:#}");
    }
    opened.is_ok()
}

fn main() -> anyhow::Result<()> {
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
            cx.set_menus(vec![Menu {
                name: "Wisp Science".into(),
                items: vec![
                    MenuItem::action("New Window", NewWindow),
                    MenuItem::separator(),
                    MenuItem::action("Quit", Quit),
                ],
            }]);
            if !open_window(ui_backend, cx) {
                cx.quit();
            }
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
