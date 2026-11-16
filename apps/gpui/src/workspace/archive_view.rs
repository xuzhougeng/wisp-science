//! The research archive of the open conversation, as the WebView's review
//! has it: a dialog that has the model draft a report, its scripts and a
//! choice for each local file the conversation recorded, lets the
//! researcher change all of it, and freezes it; the files chosen for
//! deletion are then deleted for good. The host keeps the draft, so the
//! dialog reopens on it. A frozen archive is shown as it was frozen, with
//! what became of each file.
use crate::i18n::{fill, tr};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};
use wisp_server::desktop::{ArchiveScript, ResearchArchive};

use super::{
    Workspace,
    shell::error_banner,
    surfaces::{self, DialogWidth},
};
use crate::{
    archive::{self, FileAction, Leaving, Review},
    backend::receive,
    layers::Layer,
};

/// Where the dialog stands.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ArchiveView {
    /// Looking whether the conversation is archived already, or has a
    /// draft kept.
    Looking,
    /// The model is writing the draft.
    Drafting,
    /// The model could not write one; it can be asked again.
    NoDraft,
    /// The draft is being reviewed, or frozen.
    Reviewing(Box<Review>),
    Frozen(ResearchArchive),
}

/// What the dialog does once the host has said whether the conversation
/// has a frozen archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    /// Show it.
    Show,
    /// Open the kept draft, or have the model draft one.
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
    /// Opens the conversation's archive, or its draft, or drafts one when
    /// it has neither.
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
                        (Found::Draft, _) => this.resume_research_archive(window, cx),
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

    /// Opens the draft the host kept, as the WebView does, or has the
    /// model write one when there is none.
    fn resume_research_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let reply = self
            .backend
            .research_archive_draft(project, session.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.archive_is_for(&session, &ArchiveView::Looking) {
                    return;
                }
                match result {
                    Ok(Some(kept)) => this.review_research_archive(kept, window, cx),
                    Ok(None) | Err(_) => this.draft_research_archive(window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The dialog is still open on `session` and still at `step`.
    fn archive_is_for(&self, session: &str, step: &ArchiveView) -> bool {
        self.layers.contains(Layer::ResearchArchive)
            && self.state.session.as_deref() == Some(session)
            && self.archive.as_ref() == Some(step)
    }

    /// Has the model write the draft, which the host keeps in place of an
    /// earlier one, and puts it in the fields.
    fn draft_research_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.layers.close(Layer::ConfirmDiscardArchive);
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
                match result {
                    Ok(draft) => this.review_research_archive(draft, window, cx),
                    Err(error) => {
                        this.archive_error = Some(error);
                        this.archive = Some(ArchiveView::NoDraft);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Puts `draft` in the fields, its first script in the editor.
    fn review_research_archive(
        &mut self,
        draft: ResearchArchive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The title's field is one line, whatever the model wrote.
        let title = crate::present::one_line(&draft.title);
        // The field's line endings are plain newlines.
        let report = crate::files::editor_text(&draft.report);
        for (field, text) in self.archive_fields.iter().zip([&title, &report]) {
            field.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
        }
        let review = Review::new(draft, title, report);
        let first = review.shown.and_then(|at| review.scripts.get(at).cloned());
        self.archive = Some(ArchiveView::Reviewing(Box::new(review)));
        if let Some(script) = first {
            self.load_archive_script(&script, window, cx);
        }
    }

    fn load_archive_script(
        &mut self,
        script: &ArchiveScript,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.archive_fields[2].update(cx, |input, cx| {
            input.set_value(script.filename.clone(), window, cx)
        });
        self.archive_fields[3].update(cx, |input, cx| {
            input.set_highlighter(archive::highlighter(&script.filename), cx);
            input.set_value(script.content.clone(), window, cx);
        });
    }

    /// Opens another of the draft's scripts; the one shown keeps its
    /// changes in the draft.
    fn show_archive_script(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.note_archive_edit(cx);
        let Some(ArchiveView::Reviewing(review)) = self.archive.as_mut() else {
            return;
        };
        let Some(script) = review.show(index).cloned() else {
            return;
        };
        self.load_archive_script(&script, window, cx);
        cx.notify();
    }

    /// The draft's title and report as they are in their fields.
    fn archive_text(&self, cx: &App) -> (String, String) {
        let [title, report] = [0, 1].map(|at| self.archive_fields[at].read(cx).value().to_string());
        (title, report)
    }

    /// Keeps what the script editor holds as the shown script.
    pub(super) fn note_archive_edit(&mut self, cx: &mut Context<Self>) {
        let [name, text] = [2, 3].map(|at| self.archive_fields[at].read(cx).value().to_string());
        if let Some(ArchiveView::Reviewing(review)) = self.archive.as_mut() {
            if !review.freezing {
                review.note_script(&name, &text);
            }
            cx.notify();
        }
    }

    fn choose_archive_file(&mut self, index: usize, action: FileAction, cx: &mut Context<Self>) {
        if let Some(ArchiveView::Reviewing(review)) = self.archive.as_mut() {
            if !review.freezing && review.choose(index, action) {
                cx.notify();
            }
        }
    }

    fn accept_archive(&mut self, read: bool, cx: &mut Context<Self>) {
        let (title, report) = self.archive_text(cx);
        if let Some(ArchiveView::Reviewing(review)) = self.archive.as_mut() {
            review.accept(&title, &report, read);
            cx.notify();
        }
    }

    /// Freezes the draft as reviewed. The conversation is read-only after,
    /// and the files chosen for deletion are deleted for good.
    fn freeze_research_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (title, report) = self.archive_text(cx);
        let (Some(project), Some(session), Some(ArchiveView::Reviewing(review))) = (
            self.state.project.clone(),
            self.state.session.clone(),
            self.archive.as_mut(),
        ) else {
            return;
        };
        if !review.ready(&title, &report) {
            return;
        }
        let input = review.request(&title, &report);
        let id = input.id.clone();
        review.freezing = true;
        self.archive_error = None;
        let reply = self
            .backend
            .freeze_research_archive(project.clone(), session.clone(), input);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            // A deletion can fail after the freeze; the archive is then
            // frozen all the same, with its receipts.
            let after = match &result {
                Ok(_) => None,
                Err(_) => Some(
                    receive(this.update(cx, |this, _| {
                        this.backend.research_archive(project, session.clone())
                    })?)
                    .await,
                ),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let freezing = matches!(
                    &this.archive,
                    Some(ArchiveView::Reviewing(review)) if review.freezing && review.draft.id == id
                );
                if !freezing
                    || !this.layers.contains(Layer::ResearchArchive)
                    || this.state.session.as_deref() != Some(session.as_str())
                {
                    return;
                }
                let frozen = match (result, after) {
                    (Ok(frozen), _) => Some(frozen),
                    (Err(error), after) => {
                        this.archive_error = Some(error);
                        after.and_then(Result::ok).flatten()
                    }
                };
                match frozen {
                    Some(frozen) => {
                        this.layers.close(Layer::ConfirmDiscardArchive);
                        this.archive = Some(ArchiveView::Frozen(frozen));
                        // The conversation now reads as a record.
                        this.start_poll(window, cx);
                    }
                    None => {
                        if let Some(ArchiveView::Reviewing(review)) = this.archive.as_mut() {
                            review.freezing = false;
                        }
                    }
                }
                cx.notify();
            });
            anyhow::Ok(())
        })
        .detach();
        cx.notify();
    }

    /// Tries again to delete what the frozen archive chose to delete.
    fn retry_archive_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.archive_error = None;
        let reply = self
            .backend
            .retry_research_archive_cleanup(project, session.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, _, cx| {
                if !this.layers.contains(Layer::ResearchArchive)
                    || this.state.session.as_deref() != Some(session.as_str())
                    || !matches!(this.archive, Some(ArchiveView::Frozen(_)))
                {
                    return;
                }
                match result {
                    Ok(frozen) => this.archive = Some(ArchiveView::Frozen(frozen)),
                    Err(error) => this.archive_error = Some(error),
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

    /// Shows a file the frozen archive saved, in the Files tab.
    fn open_archive_material(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close_research_archive(window, cx);
        self.reveal_file(path, window, cx);
    }

    /// Whether leaving now would give up changes to the draft.
    pub(super) fn archive_needs_asking(&self, cx: &App) -> bool {
        let (title, report) = self.archive_text(cx);
        matches!(
            &self.archive,
            Some(ArchiveView::Reviewing(review)) if !review.freezing && review.edited(&title, &report)
        )
    }

    pub(super) fn ask_before_leaving_archive(&mut self, leaving: Leaving, cx: &mut Context<Self>) {
        if let Some(ArchiveView::Reviewing(review)) = self.archive.as_mut() {
            review.asking = Some(leaving);
            self.layers.open(Layer::ConfirmDiscardArchive);
            cx.notify();
        }
    }

    pub(super) fn stop_asking_about_archive(&mut self) {
        if let Some(ArchiveView::Reviewing(review)) = self.archive.as_mut() {
            review.asking = None;
        }
    }

    /// Closes the dialog or drafts again, asking first when the draft has
    /// changes that would be given up.
    fn leave_research_archive(
        &mut self,
        leaving: Leaving,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.archive_needs_asking(cx) {
            self.ask_before_leaving_archive(leaving, cx);
            return;
        }
        match leaving {
            Leaving::Close => self.close_research_archive(window, cx),
            Leaving::Redraft => self.draft_research_archive(window, cx),
        }
    }

    /// Gives up the draft's changes, as the question asked.
    fn discard_archive_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let leaving = match self.archive.as_mut() {
            Some(ArchiveView::Reviewing(review)) => review.asking.take(),
            _ => None,
        };
        self.layers.close(Layer::ConfirmDiscardArchive);
        match leaving {
            Some(Leaving::Close) => self.close_research_archive(window, cx),
            Some(Leaving::Redraft) => self.draft_research_archive(window, cx),
            None => {}
        }
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

    /// The draft's scripts: one to choose at a time, its name and its
    /// text in the code editor.
    fn archive_scripts(&self, review: &Review, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let note = |text: &str| div().text_xs().text_color(muted).child(text.to_owned());
        let mut part = div()
            .v_flex()
            .gap_2()
            .child(note(tr("Recorded operations")));
        if review.scripts.is_empty() {
            return part.child(note(tr("No scripts go with it.")));
        }
        let mut names = div().h_flex().flex_wrap().gap_1();
        for (index, script) in review.scripts.iter().enumerate() {
            names = names.child(
                Button::new(("archive-script", index))
                    .label(script.filename.clone())
                    .xsmall()
                    .ghost()
                    .selected(review.shown == Some(index))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.show_archive_script(index, window, cx)
                    })),
            );
        }
        part = part.child(names);
        if review.shown.is_some() {
            part = part
                .child(note(tr("Script filename")))
                .child(Input::new(&self.archive_fields[2]).small())
                .child(note(tr("Script content")))
                .child(
                    div()
                        .h(px(240.))
                        .child(Input::new(&self.archive_fields[3]).h_full()),
                );
        }
        part.child(note(tr(
            "Changes stay in the draft until it is frozen; nothing is written before.",
        )))
    }

    /// The local files the conversation recorded, each with its choice,
    /// and what freezing will delete for good, by name.
    fn archive_files(&self, review: &Review, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let chinese = crate::i18n::current() == crate::i18n::Locale::Zh;
        let note = |text: String| div().text_xs().text_color(muted).child(text);
        let mut part = div()
            .v_flex()
            .gap_2()
            .child(note(tr("Local materials").into()))
            .child(note(
                tr("Snapshots preserve the reviewed bytes. References keep the original location. Remote files are excluded.")
                    .into(),
            ));
        if review.draft.files.is_empty() {
            part = part.child(note(tr("The conversation recorded no local files.").into()));
        }
        for (index, (file, chosen)) in review.draft.files.iter().zip(&review.actions).enumerate() {
            let mut choices = div().h_flex().flex_wrap().gap_1();
            for action in FileAction::ALL {
                let mut button = Button::new(SharedString::from(format!(
                    "archive-file-{index}-{}",
                    action.key()
                )))
                .label(action.label())
                .xsmall()
                .ghost()
                .selected(*chosen == action)
                .disabled(review.freezing || (action == FileAction::Delete && !file.can_delete))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.choose_archive_file(index, action, cx)),
                );
                if action == FileAction::Delete && !file.can_delete {
                    button = button.tooltip(tr(
                        "Only a file this conversation created, that nothing else uses, can be deleted.",
                    ));
                }
                choices = choices.child(button);
            }
            part = part.child(
                div()
                    .v_flex()
                    .gap_1()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(file.path.clone()),
                    )
                    .child(note(format!(
                        "{} · {}",
                        crate::files::size_label(file.size_bytes),
                        archive::bilingual(&file.reason, chinese)
                    )))
                    .child(choices),
            );
        }
        // What is deleted, said plainly before Freeze can be pressed.
        let deleted = review.deletions();
        part = part.child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(review.deletion_line()),
        );
        if !deleted.is_empty() {
            let mut names = div().v_flex().gap_0p5();
            for file in deleted {
                names = names.child(note(file.path.clone()));
            }
            part = part.child(names).child(note(
                tr("They are deleted for good once the archive is frozen. There is no trash to bring them back from.")
                    .into(),
            ));
        }
        for warning in &review.draft.warnings {
            part = part.child(note(archive::bilingual(warning, chinese).to_owned()));
        }
        part
    }

    /// The files of a frozen archive and what became of each.
    fn frozen_files(&self, frozen: &ResearchArchive, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let note = |text: String| div().text_xs().text_color(muted).child(text);
        let mut part = div().v_flex().gap_2();
        if frozen.files.is_empty() {
            return part;
        }
        part = part.child(note(tr("Local materials").into()));
        for (index, file) in frozen.files.iter().enumerate() {
            let mut said = vec![FileAction::of(&file.action).label().to_owned()];
            said.extend(archive::cleanup_line(&file.cleanup_status));
            let mut row = div().h_flex().gap_2().child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().child(file.path.clone()))
                    .child(note(said.join(" · "))),
            );
            if let Some(saved) = file.snapshot_path.clone() {
                row = row.child(
                    Button::new(("archive-material", index))
                        .label(tr("Open saved material"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_archive_material(saved.clone(), window, cx)
                        })),
                );
            }
            part = part.child(row);
        }
        part
    }

    /// Has the model write the draft again, asking first when the one
    /// open was changed.
    fn archive_redraft(busy: bool, cx: &mut Context<Self>) -> Button {
        Button::new("archive-redraft")
            .label(tr("Draft again"))
            .small()
            .ghost()
            .disabled(busy)
            .on_click(cx.listener(|this, _, window, cx| {
                this.leave_research_archive(Leaving::Redraft, window, cx)
            }))
    }

    pub(super) fn render_research_archive(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let note = |text: String| div().text_xs().text_color(muted).child(text);
        // A click beside the dialog does not throw a draft away: only Close
        // and Escape leave it, and they ask first when it was changed.
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
                false => fill(tr("Scripts that go with it: {}"), &[&names.join(", ")]),
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
            Some(ArchiveView::NoDraft) => {
                card = card.child(note(
                    tr("No draft could be written. Nothing was changed; ask for one again.").into(),
                ));
                card = card.action(Self::archive_redraft(false, cx));
            }
            Some(ArchiveView::Reviewing(review)) => {
                let (title, report) = self.archive_text(cx);
                card = card
                    .child(note(
                        tr(
                            "Read the draft and change what is not right: it is your record. \
                            Freezing writes the report, the scripts and the snapshots into the \
                            project and makes this conversation read-only, then deletes the \
                            files chosen for deletion for good; work goes on in a new one.",
                        )
                        .into(),
                    ))
                    .child(note(tr("Title").into()))
                    .child(Input::new(&self.archive_fields[0]).small())
                    .child(note(tr("Report, in Markdown").into()))
                    .child(
                        div()
                            .h(px(300.))
                            .child(Input::new(&self.archive_fields[1]).h_full()),
                    )
                    .child(self.archive_scripts(review, cx))
                    .child(self.archive_files(review, cx));
                if let Some(problem) = review.problem(&title, &report) {
                    card = card.child(note(problem));
                }
                let accepted = review.accepted(&title, &report);
                card = card.child(
                    div()
                        .h_flex()
                        .gap_3()
                        .items_center()
                        .child(div().flex_1().text_sm().child(tr(
                            "I reviewed the materials. This makes the conversation read-only \
                             and permanently deletes the selected files.",
                        )))
                        .child(
                            surfaces::switch(
                                "archive-consent",
                                accepted,
                                tr("I reviewed the materials"),
                            )
                            .disabled(review.freezing)
                            .on_change(cx.listener(
                                |this, read: &bool, _, cx| this.accept_archive(*read, cx),
                            )),
                        ),
                );
                card = card.action(Self::archive_redraft(review.freezing, cx));
                main = Some(
                    Button::new("archive-freeze")
                        .label(if review.freezing {
                            tr("Working…")
                        } else {
                            tr("Freeze")
                        })
                        .small()
                        .primary()
                        .disabled(!review.ready(&title, &report))
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
                    .child(note(fill(
                        tr("Frozen. The report is in the project at {}. This conversation is read-only."),
                        &[&format!(".wisp/research-archives/{}/REPORT.md", frozen.id)],
                    )))
                    .child(
                        TextView::markdown("archive-report", frozen.report.clone(), window, cx)
                            .selectable(true),
                    )
                    .child(note(scripts_line(&frozen.scripts)))
                    .child(self.frozen_files(frozen, cx));
                if archive::cleanup_incomplete(frozen) {
                    card = card.action(
                        Button::new("archive-retry-cleanup")
                            .label(tr("Retry incomplete cleanup"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.retry_archive_cleanup(window, cx)
                            })),
                    );
                }
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
                .on_click(cx.listener(|this, _, window, cx| {
                    this.leave_research_archive(Leaving::Close, window, cx)
                })),
        )
        .actions(main)
    }

    /// The question before the draft's changes are given up.
    pub(super) fn render_discard_archive(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let redraft = matches!(
            &self.archive,
            Some(ArchiveView::Reviewing(review)) if review.asking == Some(Leaving::Redraft)
        );
        surfaces::dialog(
            "discard-archive",
            DialogWidth::Narrow,
            tr("Give up the changes to this draft?"),
        )
        .child(div().text_sm().child(if redraft {
            tr("Your changes to the title, report, scripts and file choices are not kept: the model writes a new draft in place of this one.")
        } else {
            tr("Your changes to the title, report, scripts and file choices are not kept. The draft the model wrote stays, to review another time.")
        }))
        .action(
            Button::new("discard-archive-keep")
                .label(tr("Keep editing"))
                .small()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::ConfirmDiscardArchive);
                    this.stop_asking_about_archive();
                    cx.notify();
                })),
        )
        .action(
            Button::new("discard-archive-confirm")
                .label(if redraft {
                    tr("Draft again")
                } else {
                    tr("Give up changes")
                })
                .small()
                .danger()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.discard_archive_changes(window, cx)
                })),
        )
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

    #[test]
    fn leaving_a_changed_draft_asks_first() {
        let source = include_str!("archive_view.rs");
        let close = source.find("Button::new(\"archive-close\")").unwrap();
        let close = &source[close..close + 400];
        assert!(close.contains("leave_research_archive(Leaving::Close"));
        assert!(!close.contains("close_research_archive("));
        // Escape asks too, before the dialog goes.
        let workspace = include_str!("../workspace.rs");
        let escape = workspace.find("Some(Layer::ResearchArchive) => {").unwrap();
        let escape = &workspace[escape..escape + 400];
        assert!(escape.contains("archive_needs_asking"));
        assert!(escape.contains("ask_before_leaving_archive(archive::Leaving::Close"));
    }
}
