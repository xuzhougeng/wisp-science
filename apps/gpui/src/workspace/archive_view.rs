//! The research archive of the open conversation: a dialog that has the
//! model draft a report, lets the researcher change it, and freezes it.
//! A frozen archive is shown as it was frozen.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};
use wisp_server::desktop::{ArchiveDraft, ArchiveScript, ResearchArchive};

use super::{
    Workspace,
    shell::error_banner,
    surfaces::{self, DialogWidth},
};
use crate::{backend::receive, layers::Layer};

/// Where the dialog stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ArchiveView {
    /// Looking whether the conversation is archived already.
    Looking,
    /// The model is writing the draft.
    Drafting,
    /// The draft is in the fields; these scripts go with it.
    Reviewing(Vec<ArchiveScript>),
    Freezing(Vec<ArchiveScript>),
    Frozen(ResearchArchive),
}

/// What the dialog does once the host has said whether the conversation
/// has a frozen archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    /// Show it.
    Show,
    /// Have the model draft one.
    Draft,
    /// Close: there is none, and none is to be drafted.
    Close,
}

/// `may_draft` is whether the dialog was opened to make an archive (the
/// conversation's Research archive button, `/archive`, the palette) rather
/// than to look at one (a read-only conversation's card).
fn after_looking(frozen: bool, may_draft: bool) -> Found {
    match (frozen, may_draft) {
        (true, _) => Found::Show,
        (false, true) => Found::Draft,
        (false, false) => Found::Close,
    }
}

impl Workspace {
    /// Opens the conversation's archive, or drafts one when it has none.
    pub(super) fn open_research_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.look_for_research_archive(true, window, cx);
    }

    /// Opens the conversation's frozen archive, and only that: with none
    /// the dialog closes again and nothing is drafted.
    pub(super) fn open_frozen_research_archive(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.look_for_research_archive(false, window, cx);
    }

    fn look_for_research_archive(
        &mut self,
        may_draft: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::ResearchArchive);
        self.archive = Some(ArchiveView::Looking);
        self.archive_error = None;
        let reply = self.backend.research_archive(project, session.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.archive_is_for(&session, &ArchiveView::Looking) {
                    return;
                }
                match result {
                    Ok(found) => match (after_looking(found.is_some(), may_draft), found) {
                        (Found::Show, Some(frozen)) => {
                            this.archive = Some(ArchiveView::Frozen(frozen))
                        }
                        (Found::Draft, _) => this.draft_research_archive(window, cx),
                        _ => {
                            this.close_research_archive(window, cx);
                            this.error =
                                Some(tr("This conversation has no research archive.").into());
                        }
                    },
                    Err(error) => this.archive_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The dialog is still open on `session` and still at `step`.
    fn archive_is_for(&self, session: &str, step: &ArchiveView) -> bool {
        self.layers.contains(Layer::ResearchArchive)
            && self.state.session.as_deref() == Some(session)
            && self.archive.as_ref() == Some(step)
    }

    /// Has the model write the draft and puts it in the fields.
    fn draft_research_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.archive = Some(ArchiveView::Drafting);
        self.archive_error = None;
        let reply = self
            .backend
            .draft_research_archive(project, session.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.archive_is_for(&session, &ArchiveView::Drafting) {
                    return;
                }
                let draft = match result {
                    Ok(draft) => draft,
                    Err(error) => {
                        // The fields stay, empty, to be written by hand.
                        this.archive_error = Some(error);
                        ArchiveDraft::default()
                    }
                };
                // The title's field is one line, whatever the model wrote.
                let title = crate::present::one_line(&draft.title);
                for (field, text) in this.archive_fields.iter().zip([title, draft.report]) {
                    field.update(cx, |input, cx| input.set_value(text, window, cx));
                }
                this.archive = Some(ArchiveView::Reviewing(draft.scripts));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Freezes what is in the fields. The conversation is read-only after.
    fn freeze_research_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session), Some(ArchiveView::Reviewing(scripts))) = (
            self.state.project.clone(),
            self.state.session.clone(),
            self.archive.clone(),
        ) else {
            return;
        };
        let [title, report] = [0, 1].map(|at| self.archive_fields[at].read(cx).value().to_string());
        self.archive = Some(ArchiveView::Freezing(scripts.clone()));
        self.archive_error = None;
        let reply = self.backend.freeze_research_archive(
            project,
            session.clone(),
            ArchiveDraft {
                title,
                report,
                scripts: scripts.clone(),
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.archive_is_for(&session, &ArchiveView::Freezing(scripts.clone())) {
                    return;
                }
                match result {
                    Ok(frozen) => {
                        this.archive = Some(ArchiveView::Frozen(frozen));
                        // The conversation now reads as a record.
                        this.start_poll(window, cx);
                    }
                    Err(error) => {
                        this.archive_error = Some(error);
                        this.archive = Some(ArchiveView::Reviewing(scripts));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Goes on from the frozen archive in a new conversation, which opens.
    fn continue_research(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        self.archive_error = None;
        let reply = self.backend.continue_research(project.clone(), session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(continued) => {
                        this.close_research_archive(window, cx);
                        this.open_session(project, continued, window, cx);
                    }
                    Err(error) => this.archive_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn close_research_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::ResearchArchive) {
            self.archive = None;
            self.archive_error = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_research_archive(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let note = |text: String| div().text_xs().text_color(muted).child(text);
        // A click beside the dialog does not throw a draft away: only Close
        // and Escape leave it.
        let mut card = surfaces::dialog(
            "research-archive",
            DialogWidth::Wide,
            tr("Research archive"),
        );
        if let Some(error) = &self.archive_error {
            card = card.child(error_banner(error, cx));
        }
        // The main action, after Close at the right end.
        let mut main = None;
        let scripts_line = |scripts: &[ArchiveScript]| {
            let names: Vec<&str> = scripts
                .iter()
                .map(|script| script.filename.as_str())
                .collect();
            match names.is_empty() {
                true => tr("No scripts go with it.").to_owned(),
                false => crate::i18n::fill(tr("Scripts that go with it: {}"), &[&names.join(", ")]),
            }
        };
        match &self.archive {
            None | Some(ArchiveView::Looking) => {
                card = card.child(note(tr("Loading…").into()));
            }
            Some(ArchiveView::Drafting) => {
                card = card.child(note(
                    tr("The model is writing a draft from the conversation. This can take a while.")
                        .into(),
                ));
            }
            Some(ArchiveView::Reviewing(scripts) | ArchiveView::Freezing(scripts)) => {
                let freezing = matches!(self.archive, Some(ArchiveView::Freezing(_)));
                card = card
                    .child(note(
                        tr(
                            "Read the draft and change what is not right: it is your record. \
                            Freezing writes the report into the project and makes this \
                            conversation read-only; work goes on in a new one. No project \
                            file is moved or deleted.",
                        )
                        .into(),
                    ))
                    .child(note(tr("Title").into()))
                    .child(Input::new(&self.archive_fields[0]).small())
                    .child(note(tr("Report, in Markdown").into()))
                    .child(
                        div()
                            .h(px(340.))
                            .child(Input::new(&self.archive_fields[1]).h_full()),
                    )
                    .child(note(scripts_line(scripts)));
                card = card.action(
                    Button::new("archive-redraft")
                        .label(tr("Draft again"))
                        .small()
                        .ghost()
                        .disabled(freezing)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.draft_research_archive(window, cx)
                        })),
                );
                main = Some(
                    Button::new("archive-freeze")
                        .label(if freezing {
                            tr("Working…")
                        } else {
                            tr("Freeze")
                        })
                        .small()
                        .primary()
                        .disabled(freezing)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.freeze_research_archive(window, cx)
                        })),
                );
            }
            Some(ArchiveView::Frozen(frozen)) => {
                card = card
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(frozen.title.clone()),
                    )
                    .child(note(crate::i18n::fill(
                        tr("Frozen. The report is in the project at {}. This conversation is read-only."),
                        &[&format!(".wisp/research-archives/{}/REPORT.md", frozen.id)],
                    )))
                    .child(
                        TextView::markdown("archive-report", frozen.report.clone(), window, cx)
                            .selectable(true),
                    )
                    .child(note(scripts_line(&frozen.scripts)));
                main = Some(
                    Button::new("archive-continue")
                        .label(tr("Continue research"))
                        .small()
                        .primary()
                        .tooltip(tr("A new conversation that starts from this archive"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.continue_research(window, cx)),
                        ),
                );
            }
        }
        card.action(
            Button::new("archive-close")
                .label(tr("Close"))
                .small()
                .on_click(
                    cx.listener(|this, _, window, cx| this.close_research_archive(window, cx)),
                ),
        )
        .actions(main)
    }
}

#[cfg(test)]
mod tests {
    use super::{Found, after_looking};

    #[test]
    fn only_the_dialog_opened_to_make_an_archive_drafts_one() {
        assert_eq!(after_looking(true, true), Found::Show);
        assert_eq!(after_looking(true, false), Found::Show);
        assert_eq!(after_looking(false, true), Found::Draft);
        // Opened to look at one, it never asks the model for a draft.
        assert_eq!(after_looking(false, false), Found::Close);
    }

    #[test]
    fn the_read_only_card_only_looks_at_an_archive() {
        let shell = include_str!("shell.rs");
        let start = shell.find("fn read_only_card(").unwrap();
        let card = &shell[start..start + shell[start..].find("\n    }\n").unwrap()];
        assert!(card.contains("this.open_frozen_research_archive("));
        assert!(!card.contains("this.open_research_archive("));
    }
}
