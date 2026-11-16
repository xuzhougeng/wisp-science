//! Conversations had in Codex CLI or Claude Code on this computer, listed
//! to be brought into the open project.
use std::collections::HashMap;

use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{ExternalConversation, ImportOutcome};

use super::{Workspace, chip, shell::error_banner};
use crate::{backend::receive, layers::Layer, present::Tone};

/// The import dialog's state.
#[derive(Default)]
pub(super) struct Imports {
    /// What was found; `None` while looking.
    pub found: Option<Vec<ExternalConversation>>,
    pub error: Option<String>,
    /// The file being imported now.
    pub busy: Option<String>,
    /// What was done this time, by file: the conversation and how.
    pub done: HashMap<String, (String, ImportOutcome)>,
}

impl Workspace {
    pub(super) fn open_imports(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::ImportConversations);
        self.imports = Imports::default();
        let reply = self.backend.external_conversations(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.layers.contains(Layer::ImportConversations)
                    || this.state.project.as_deref() != Some(project.as_str())
                {
                    return;
                }
                match result {
                    Ok(found) => this.imports.found = Some(found),
                    Err(error) => {
                        this.imports.found = Some(Vec::new());
                        this.imports.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn import_one(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        if self.imports.busy.is_some() {
            return;
        }
        self.imports.busy = Some(path.clone());
        self.imports.error = None;
        let reply = self
            .backend
            .import_external_conversation(project, path.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.imports.busy = None;
                match result {
                    Ok(done) => {
                        this.imports.done.insert(path, done);
                        this.refresh_lists(window, cx);
                    }
                    Err(error) => this.imports.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn close_imports(&mut self, cx: &mut Context<Self>) {
        if self.layers.close(Layer::ImportConversations) {
            self.imports = Imports::default();
            cx.notify();
        }
    }

    pub(super) fn render_imports(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let now = chrono::Utc::now().timestamp();
        let mut card = div()
            .id("imports")
            .v_flex()
            .gap_2()
            .w(px(720.))
            .max_w(relative(0.94))
            .max_h(relative(0.86))
            .overflow_y_scroll()
            .p_5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("Import from Codex or Claude Code")),
            )
            .child(div().text_sm().text_color(muted).child(tr(
                "Conversations kept on this computer under ~/.codex/sessions and ~/.claude/projects. One goes into this project in a group named after the tool; importing it again brings in only what was added since.",
            )));
        if let Some(error) = &self.imports.error {
            card = card.child(error_banner(error, cx));
        }
        match &self.imports.found {
            None => {
                card = card.child(div().text_sm().text_color(muted).child(tr("Looking…")));
            }
            Some(found) if found.is_empty() => {
                card = card.child(div().text_sm().text_color(muted).child(tr(
                    "No Codex or Claude Code conversations were found on this computer.",
                )));
            }
            Some(found) => {
                for (index, external) in found.iter().enumerate() {
                    let path = external.path.clone();
                    let done = self.imports.done.get(&path);
                    let busy = self.imports.busy.as_deref() == Some(path.as_str());
                    let mut line = div()
                        .h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::home::age(now, external.modified_ms / 1000));
                    if !external.folder.is_empty() {
                        line = line.child(div().truncate().child(external.folder.clone()));
                    }
                    let action = match done {
                        Some((session, outcome)) => {
                            let session = session.clone();
                            let project = self.state.project.clone().unwrap_or_default();
                            Button::new(("import-open", index))
                                .label(crate::present::import_outcome(*outcome))
                                .xsmall()
                                .ghost()
                                .tooltip(tr("Open it"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.close_imports(cx);
                                    this.open_session(project.clone(), session.clone(), window, cx);
                                }))
                        }
                        None => Button::new(("import-one", index))
                            .label(if busy {
                                tr("Importing…")
                            } else if external.imported {
                                tr("Bring in what's new")
                            } else {
                                tr("Import")
                            })
                            .xsmall()
                            .disabled(self.imports.busy.is_some())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.import_one(path.clone(), window, cx)
                            })),
                    };
                    card = card.child(
                        div()
                            .h_flex()
                            .gap_3()
                            .py_2()
                            .border_b_1()
                            .border_color(border)
                            .child(chip(external.source.to_string(), Tone::Neutral, cx))
                            .child(
                                div()
                                    .v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().truncate().text_sm().child(external.title.clone()))
                                    .child(line),
                            )
                            .child(action),
                    );
                }
            }
        }
        card = card.child(
            div().h_flex().justify_end().pt_2().child(
                Button::new("imports-close")
                    .label(tr("Close"))
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.close_imports(cx))),
            ),
        );
        div()
            .id("imports-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(cx.theme().overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_imports(cx)),
            )
            .child(card)
    }
}
