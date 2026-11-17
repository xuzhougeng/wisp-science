//! The dialog that brings the WebView desktop's projects, settings and
//! lists here. Its database is found where the WebView keeps it, or
//! chosen; it is only read, as is the favorites library beside it. The
//! projects, groups of settings and lists that can come are ticked, each
//! list saying what of it would stay behind and why, and one press brings
//! them.
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{WebViewGroup, WebViewModel, WebViewOutcome, WebViewProject};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph},
    surfaces::{self, DialogWidth},
};
use crate::{backend::receive, home, layers::Layer, present::Tone, webview_import};

/// The dialog's state.
#[derive(Default)]
pub(super) struct WebViewImports {
    /// The database read, once there is one.
    pub database: Option<PathBuf>,
    /// Its projects; `None` while they are read.
    pub found: Option<Vec<WebViewProject>>,
    /// Its saved models.
    pub models: Vec<WebViewModel>,
    /// The groups of its settings that can come.
    pub settings: Vec<String>,
    /// Its lists this desktop has none of, with what of each would come.
    pub groups: Vec<WebViewGroup>,
    pub error: Option<String>,
    pub picks: BTreeSet<String>,
    pub setting_picks: BTreeSet<String>,
    pub model_picks: BTreeSet<String>,
    pub group_picks: BTreeSet<String>,
    pub busy: bool,
    /// How the last import went.
    pub done: Option<WebViewOutcome>,
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

    /// Lists the projects and settings in `database`, ticking the ones
    /// that can come.
    fn read_webview(&mut self, database: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.webview_imports.database = Some(database.clone());
        self.webview_imports.found = None;
        self.webview_imports.error = None;
        let reply = self.backend.webview_data(database.clone());
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
                    Ok(data) => {
                        imports.picks = webview_import::first_picks(&data.projects);
                        imports.setting_picks = data.settings.iter().cloned().collect();
                        imports.model_picks = webview_import::first_model_picks(&data.models);
                        imports.group_picks = webview_import::first_group_picks(&data.groups);
                        imports.models = data.models;
                        imports.found = Some(data.projects);
                        imports.settings = data.settings;
                        imports.groups = data.groups;
                    }
                    Err(error) => {
                        imports.found = Some(Vec::new());
                        imports.settings.clear();
                        imports.models.clear();
                        imports.groups.clear();
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
                    this.webview_imports.done = None;
                    this.read_webview(path, window, cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Brings the ticked projects and settings over, then lists what is
    /// left again.
    fn bring_webview_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let imports = &mut self.webview_imports;
        let (Some(database), Some(found)) = (imports.database.clone(), &imports.found) else {
            return;
        };
        let ids = webview_import::chosen(found, &imports.picks);
        // The host takes the lists among the groups.
        let settings: Vec<String> = imports
            .settings
            .iter()
            .filter(|group| imports.setting_picks.contains(*group))
            .cloned()
            .chain(webview_import::chosen_groups(
                &imports.groups,
                &imports.group_picks,
            ))
            .collect();
        let models = webview_import::chosen_models(&imports.models, &imports.model_picks);
        if imports.busy || (ids.is_empty() && settings.is_empty() && models.is_empty()) {
            return;
        }
        imports.busy = true;
        imports.error = None;
        imports.done = None;
        let reply = self
            .backend
            .import_webview(database.clone(), ids, settings, models);
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
                        this.webview_imports.done = Some(done);
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
        let mut card = surfaces::dialog(
            "webview-import",
            DialogWidth::Regular,
            tr("Bring over from the WebView desktop"),
        )
        .close_on_click_outside(cx.listener(|this, _, _, cx| {
            if !this.webview_imports.busy {
                this.close_webview_import(cx)
            }
        }))
        .child(div().text_sm().text_color(muted).child(tr(
            "The projects and settings of Wisp's WebView desktop on this computer. Its \
                 database is only read. A project kept in it is copied here, with its settings, \
                 and goes on using its folder where it is; one that keeps its own database in \
                 its folder is opened there, so both desktops then share it. A group of \
                 settings comes only while none of it is set here. A saved model comes with its \
                 key, from the keychain to the keychain.",
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
        if let Some(outcome) = &imports.done {
            card = card.child(div().text_sm().child(webview_import::summary(outcome)));
            for done in outcome.projects.iter().filter(|done| done.error.is_some()) {
                card = card.child(div().text_xs().text_color(cx.theme().danger).child(format!(
                    "{}: {}",
                    done.name,
                    done.error.as_deref().unwrap_or_default()
                )));
            }
            // What of each list chosen stayed behind, and why.
            for group in &outcome.groups {
                for (name, hold) in &group.held {
                    card = card.child(div().text_xs().text_color(muted).child(format!(
                        "{}: {}",
                        webview_import::group_label(&group.group),
                        webview_import::held_line(name, hold)
                    )));
                }
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
        if !imports.models.is_empty() {
            card = card.child(div().pt_3().text_sm().child(tr("Saved models")));
            for (index, model) in imports.models.iter().enumerate() {
                let picked = !model.here && imports.model_picks.contains(&model.id);
                let id = model.id.clone();
                let mut row = div()
                    .id(("webview-model", index))
                    .h_flex()
                    .gap_3()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .child(
                        glyph(match (model.here, picked) {
                            (true, _) => "circle-minus",
                            (false, true) => "check-square",
                            (false, false) => "circle",
                        })
                        .small(),
                    )
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().text_sm().child(model.name.clone()))
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(webview_import::model_detail(model)),
                            ),
                    );
                if model.here {
                    row = row.child(chip(tr("Already here"), Tone::Neutral, cx));
                } else {
                    row = row
                        .cursor_pointer()
                        .hover(|row| row.bg(cx.theme().list_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let picks = &mut this.webview_imports.model_picks;
                            if !picks.remove(&id) {
                                picks.insert(id.clone());
                            }
                            cx.notify();
                        }));
                }
                card = card.child(row);
            }
        }
        if !imports.groups.is_empty() {
            card = card
                .child(div().pt_3().text_sm().child(tr("More from the WebView")))
                .child(div().text_xs().text_color(muted).child(tr(
                    "Each comes only while this desktop has none of its own, and each item only \
                     as the WebView has it; what cannot come is left behind, with why.",
                )));
            for (index, group) in imports.groups.iter().enumerate() {
                let open = !group.items.is_empty();
                let picked = open && imports.group_picks.contains(&group.group);
                let key = group.group.clone();
                let mut about = div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .child(webview_import::group_label(&group.group)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(webview_import::group_detail(group)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(webview_import::group_note(&group.group)),
                    );
                for (name, hold) in &group.held {
                    about = about.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(webview_import::held_line(name, hold)),
                    );
                }
                let mut row = div()
                    .id(("webview-list", index))
                    .h_flex()
                    .items_start()
                    .gap_3()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .child(
                        glyph(match (open, picked) {
                            (false, _) => "circle-minus",
                            (true, true) => "check-square",
                            (true, false) => "circle",
                        })
                        .small(),
                    )
                    .child(about);
                if open {
                    row = row
                        .cursor_pointer()
                        .hover(|row| row.bg(cx.theme().list_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let picks = &mut this.webview_imports.group_picks;
                            if !picks.remove(&key) {
                                picks.insert(key.clone());
                            }
                            cx.notify();
                        }));
                }
                card = card.child(row);
            }
        }
        if !imports.settings.is_empty() {
            card = card.child(div().pt_3().text_sm().child(tr("Settings")));
            for (index, group) in imports.settings.iter().enumerate() {
                let picked = imports.setting_picks.contains(group);
                let group = group.clone();
                card = card.child(
                    div()
                        .id(("webview-setting", index))
                        .h_flex()
                        .gap_3()
                        .px_2()
                        .py_1()
                        .rounded(cx.theme().radius)
                        .cursor_pointer()
                        .hover(|row| row.bg(cx.theme().list_hover))
                        .child(glyph(if picked { "check-square" } else { "circle" }).small())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(webview_import::setting_label(&group)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let picks = &mut this.webview_imports.setting_picks;
                            if !picks.remove(&group) {
                                picks.insert(group.clone());
                            }
                            cx.notify();
                        })),
                );
            }
        }
        let chosen = imports.found.as_deref().map_or(0, |found| {
            webview_import::chosen(found, &imports.picks).len()
        }) + imports
            .settings
            .iter()
            .filter(|group| imports.setting_picks.contains(*group))
            .count()
            + webview_import::chosen_models(&imports.models, &imports.model_picks).len()
            + webview_import::chosen_groups(&imports.groups, &imports.group_picks).len();
        let can_come = !imports.settings.is_empty()
            || imports.models.iter().any(|model| !model.here)
            || imports.groups.iter().any(|group| !group.items.is_empty())
            || imports
                .found
                .as_deref()
                .is_some_and(|found| !webview_import::first_picks(found).is_empty());
        card.when(can_come, |card| {
            card.aside(
                Button::new("webview-import-all")
                    .label(tr("Select all"))
                    .small()
                    .ghost()
                    .disabled(imports.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let imports = &mut this.webview_imports;
                        if let Some(found) = &imports.found {
                            imports.picks = webview_import::first_picks(found);
                        }
                        imports.setting_picks = imports.settings.iter().cloned().collect();
                        imports.model_picks = webview_import::first_model_picks(&imports.models);
                        imports.group_picks = webview_import::first_group_picks(&imports.groups);
                        cx.notify();
                    })),
            )
        })
        .action(
            Button::new("webview-import-close")
                .label(tr("Close"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| this.close_webview_import(cx))),
        )
        .when(can_come, |card| {
            card.action(
                Button::new("webview-import-bring")
                    .label(if imports.busy {
                        tr("Bringing over…").to_owned()
                    } else {
                        crate::i18n::fill(tr("Bring over {}"), &[&chosen])
                    })
                    .small()
                    .primary()
                    .disabled(imports.busy || chosen == 0)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.bring_webview_projects(window, cx)),
                    ),
            )
        })
    }
}
