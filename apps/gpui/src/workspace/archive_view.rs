//! The research archive of the open conversation: a dialog that has the
//! model draft a report, lets the researcher change it, and freezes it.
//! A frozen archive is shown as it was frozen.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};
use wisp_server::desktop::{ArchiveDraft, ArchiveScript, ResearchArchive};

use super::{Workspace, shell::error_banner};
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

impl Workspace {
    pub(super) fn open_research_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                    Ok(Some(frozen)) => this.archive = Some(ArchiveView::Frozen(frozen)),
                    Ok(None) => this.draft_research_archive(window, cx),
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
        let border = cx.theme().border;
        let note = |text: String| div().text_xs().text_color(muted).child(text);
        let mut card = div()
            .id("research-archive")
            .v_flex()
            .gap_2()
            .w(px(760.))
            .max_w(relative(0.94))
            .max_h(relative(0.92))
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
                    .child(tr("Research archive")),
            );
        if let Some(error) = &self.archive_error {
            card = card.child(error_banner(error, cx));
        }
        let mut actions = div().h_flex().gap_2().justify_end().pt_2();
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
                actions = actions
                    .child(
                        Button::new("archive-redraft")
                            .label(tr("Draft again"))
                            .small()
                            .ghost()
                            .disabled(freezing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.draft_research_archive(window, cx)
                            })),
                    )
                    .child(
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
            }
        }
        actions = actions.child(
            Button::new("archive-close")
                .label(tr("Close"))
                .small()
                .on_click(
                    cx.listener(|this, _, window, cx| this.close_research_archive(window, cx)),
                ),
        );
        div()
            .id("research-archive-backdrop")
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
            // A click beside the dialog does not throw a draft away: only
            // Close and Escape leave it.
            .child(card.child(actions))
    }
}
