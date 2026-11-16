//! The dialog that brings the WebView desktop's projects here. Its
//! database is found where the WebView keeps it, or chosen; it is only
//! read. The projects that can come are ticked, and one press brings them.
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{WebViewImport, WebViewProject};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph},
};
use crate::{backend::receive, home, layers::Layer, present::Tone, webview_import};

/// The dialog's state.
#[derive(Default)]
pub(super) struct WebViewImports {
    /// The database read, once there is one.
    pub database: Option<PathBuf>,
    /// Its projects; `None` while they are read.
    pub found: Option<Vec<WebViewProject>>,
    pub error: Option<String>,
    pub picks: BTreeSet<String>,
    pub busy: bool,
    /// How the last import went, project by project.
    pub done: Vec<WebViewImport>,
}

impl Workspace {
    /// Opens the dialog on the WebView's database where it keeps it.
    pub(super) fn open_webview_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::WebViewImport);
        self.webview_imports = WebViewImports::default();
        match webview_import::database().filter(|path| path.is_file()) {
            Some(database) => self.read_webview(database, window, cx),
            None => self.webview_imports.found = Some(Vec::new()),
        }
        cx.notify();
    }

    /// Lists the projects in `database`, ticking the ones that can come.
    fn read_webview(&mut self, database: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.webview_imports.database = Some(database.clone());
        self.webview_imports.found = None;
        self.webview_imports.error = None;
        let reply = self.backend.webview_projects(database.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                let imports = &mut this.webview_imports;
                if !this.layers.contains(Layer::WebViewImport)
                    || imports.database.as_ref() != Some(&database)
                {
                    return;
                }
                match result {
                    Ok(found) => {
                        imports.picks = webview_import::first_picks(&found);
                        imports.found = Some(found);
                    }
                    Err(error) => {
                        imports.found = Some(Vec::new());
                        imports.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks for another database file, in the system's dialog.
    fn choose_webview_database(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Read").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.layers.contains(Layer::WebViewImport) {
                    this.webview_imports.done.clear();
                    this.read_webview(path, window, cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Brings the ticked projects over, then lists them again.
    fn bring_webview_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let imports = &mut self.webview_imports;
        let (Some(database), Some(found)) = (imports.database.clone(), &imports.found) else {
            return;
        };
        let ids = webview_import::chosen(found, &imports.picks);
        if imports.busy || ids.is_empty() {
            return;
        }
        imports.busy = true;
        imports.error = None;
        imports.done.clear();
        let reply = self.backend.import_webview_projects(database.clone(), ids);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.layers.contains(Layer::WebViewImport) {
                    // Closed meanwhile: the home page still shows them.
                    this.load_projects(window, cx);
                    this.load_recent(window, cx);
                    return;
                }
                this.webview_imports.busy = false;
                match result {
                    Ok(done) => {
                        this.webview_imports.done = done;
                        this.read_webview(database, window, cx);
                        this.load_projects(window, cx);
                        this.load_recent(window, cx);
                    }
                    Err(error) => this.webview_imports.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn close_webview_import(&mut self, cx: &mut Context<Self>) {
        if self.layers.close(Layer::WebViewImport) {
            self.webview_imports = WebViewImports::default();
            cx.notify();
        }
    }

    pub(super) fn render_webview_import(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let imports = &self.webview_imports;
        let home_dir = dirs::home_dir().map(|path| path.to_string_lossy().into_owned());
        let mut card = div()
            .id("webview-import")
            .v_flex()
            .gap_2()
            .w(px(640.))
            .max_w(relative(0.94))
            .max_h(relative(0.86))
            .overflow_y_scroll()
            .p_5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(border)
            .bg(cx.theme().popover)
            .shadow_lg()
            // A click inside is not a click on the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("Bring over from the WebView desktop")),
            )
            .child(div().text_sm().text_color(muted).child(tr(
                "The projects of Wisp's WebView desktop on this computer. Its database is \
                 only read. A project kept in it is copied here and goes on using its folder \
                 where it is; one that keeps its own database in its folder is opened there, \
                 so both desktops then share it. Settings and model keys stay behind.",
            )));
        let place = imports.database.as_ref().map_or_else(
            || tr("No WebView data was found on this computer.").to_owned(),
            |path| home::short_path(&path.to_string_lossy(), home_dir.as_deref()),
        );
        card = card.child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(place),
                )
                .child(
                    Button::new("webview-import-choose")
                        .label(tr("Choose its database…"))
                        .xsmall()
                        .ghost()
                        .disabled(imports.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.choose_webview_database(window, cx)
                        })),
                ),
        );
        if let Some(error) = &imports.error {
            card = card.child(error_banner(error, cx));
        }
        if !imports.done.is_empty() {
            card = card.child(
                div()
                    .text_sm()
                    .child(webview_import::summary(&imports.done)),
            );
            for done in imports.done.iter().filter(|done| done.error.is_some()) {
                card = card.child(div().text_xs().text_color(cx.theme().danger).child(format!(
                    "{}: {}",
                    done.name,
                    done.error.as_deref().unwrap_or_default()
                )));
            }
        }
        match &imports.found {
            None => {
                card = card.child(div().text_sm().text_color(muted).child(tr("Looking…")));
            }
            Some(found) if found.is_empty() => {
                if imports.database.is_some() && imports.error.is_none() {
                    card = card.child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(tr("The WebView has no projects.")),
                    );
                }
            }
            Some(found) => {
                for (index, project) in found.iter().enumerate() {
                    let hold = webview_import::hold(project);
                    let picked = hold.is_none() && imports.picks.contains(&project.id);
                    let id = project.id.clone();
                    let mut line = div().h_flex().gap_1().text_xs().text_color(muted).child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(home::short_path(&project.folder, home_dir.as_deref())),
                    );
                    let detail = webview_import::detail(project);
                    if !detail.is_empty() {
                        line = line.child("·").child(div().flex_shrink_0().child(detail));
                    }
                    let mut row = div()
                        .id(("webview-project", index))
                        .h_flex()
                        .gap_3()
                        .px_2()
                        .py_2()
                        .border_b_1()
                        .border_color(border)
                        .child(
                            glyph(match (hold, picked) {
                                (Some(_), _) => "circle-minus",
                                (None, true) => "check-square",
                                (None, false) => "circle",
                            })
                            .small(),
                        )
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .h_flex()
                                        .gap_1()
                                        .text_sm()
                                        .child(div().truncate().child(project.name.clone()))
                                        .when(project.starred, |name| {
                                            name.child(glyph("star-filled").xsmall())
                                        }),
                                )
                                .child(line),
                        );
                    if let Some(hold) = hold {
                        row = row.child(chip(hold.to_owned(), Tone::Neutral, cx));
                    } else {
                        row = row
                            .cursor_pointer()
                            .hover(|row| row.bg(cx.theme().list_hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let picks = &mut this.webview_imports.picks;
                                if !picks.remove(&id) {
                                    picks.insert(id.clone());
                                }
                                cx.notify();
                            }));
                    }
                    card = card.child(row);
                }
            }
        }
        let chosen = imports.found.as_deref().map_or(0, |found| {
            webview_import::chosen(found, &imports.picks).len()
        });
        let can_come = imports
            .found
            .as_deref()
            .is_some_and(|found| !webview_import::first_picks(found).is_empty());
        card = card.child(
            div()
                .h_flex()
                .gap_2()
                .pt_2()
                .when(can_come, |row| {
                    row.child(
                        Button::new("webview-import-all")
                            .label(tr("Select all"))
                            .small()
                            .ghost()
                            .disabled(imports.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(found) = &this.webview_imports.found {
                                    this.webview_imports.picks = webview_import::first_picks(found);
                                }
                                cx.notify();
                            })),
                    )
                })
                .child(div().flex_1())
                .child(
                    Button::new("webview-import-close")
                        .label(tr("Close"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.close_webview_import(cx))),
                )
                .when(can_come, |row| {
                    row.child(
                        Button::new("webview-import-bring")
                            .label(if imports.busy {
                                tr("Bringing over…").to_owned()
                            } else {
                                crate::i18n::fill(tr("Bring over {}"), &[&chosen])
                            })
                            .small()
                            .primary()
                            .disabled(imports.busy || chosen == 0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.bring_webview_projects(window, cx)
                            })),
                    )
                }),
        );
        div()
            .id("webview-import-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            // Nothing underneath is clickable while the dialog is open.
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(cx.theme().overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if !this.webview_imports.busy {
                        this.close_webview_import(cx)
                    }
                }),
            )
            .child(card)
    }
}
