#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod backend;
mod state;
mod workspace;

use gpui::{prelude::*, *};
use gpui_component::Root;
use std::sync::Arc;

actions!(wisp, [Quit]);

fn main() -> anyhow::Result<()> {
    let runtime = Arc::new(tokio::runtime::Runtime::new()?);
    let backend = backend::Backend::new(runtime.clone());
    let ui_backend = backend.clone();
    let quit_backend = backend.clone();
    let quit_runtime = runtime.clone();
    Application::new().run(move |cx| {
        gpui_component::init(cx);
        cx.on_app_quit(move |_| {
            // GPUI gives returned futures only 100ms and macOS terminates the
            // process without returning from Application::run. Finish the host's
            // bounded shutdown here, before allowing native termination.
            quit_runtime.block_on(quit_backend.shutdown());
            async {}
        })
        .detach();
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-q"
            } else {
                "ctrl-q"
            },
            Quit,
            None,
        )]);
        cx.set_menus(vec![Menu {
            name: "Wisp Science".into(),
            items: vec![MenuItem::action("Quit", Quit)],
        }]);
        let bounds = Bounds::centered(None, size(px(1180.), px(780.)), cx);
        if let Err(error) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(820.), px(540.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Wisp Science · GPUI Preview".into()),
                    ..Default::default()
                }),
                app_id: Some("science.wisp.gpui".into()),
                ..Default::default()
            },
            |window, cx| {
                let workspace = cx.new(|cx| workspace::Workspace::new(ui_backend, window, cx));
                cx.new(|cx| Root::new(workspace, window, cx))
            },
        ) {
            eprintln!("Cannot open GPUI window: {error:#}");
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
