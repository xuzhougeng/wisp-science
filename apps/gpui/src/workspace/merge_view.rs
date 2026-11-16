//! Bringing a branch back into the conversation it came from: the model
//! summarises what the branch did after it split off, the user reads and
//! changes the summary, and it is added there as one message.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::BranchMerge;

use super::{
    Workspace,
    shell::error_banner,
    surfaces::{self, DialogWidth},
};
use crate::{backend::receive, layers::Layer};

/// Where the dialog stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MergeView {
    /// Looking at the branch.
    Looking,
    /// What it did is known; nothing is written yet.
    Ready(BranchMerge),
    /// The model is writing the summary.
    Drafting(BranchMerge),
    /// The summary is in its field, to be read and changed.
    Reviewing(BranchMerge),
    Merging(BranchMerge),
}

impl MergeView {
    fn branch(&self) -> Option<&BranchMerge> {
        match self {
            MergeView::Looking => None,
            MergeView::Ready(found)
            | MergeView::Drafting(found)
            | MergeView::Reviewing(found)
            | MergeView::Merging(found) => Some(found),
        }
    }
}

impl Workspace {
    /// Whether the open conversation is a branch that could be brought back.
    pub(super) fn open_is_branch(&self) -> bool {
        self.state
            .session
            .as_ref()
            .is_some_and(|session| self.branched.contains_key(session))
    }

    pub(super) fn open_branch_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(branch)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::BranchMerge);
        self.merge = Some(MergeView::Looking);
        self.merge_error = None;
        self.merge_summary
            .update(cx, |input, cx| input.set_value("", window, cx));
        let reply = self.backend.branch_merge(project, branch.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, _, cx| {
                if !this.merge_is_for(&branch, &MergeView::Looking) {
                    return;
                }
                match result {
                    Ok(found) => this.merge = Some(MergeView::Ready(found)),
                    Err(error) => this.merge_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The dialog is still open on `branch` and still at `step`.
    fn merge_is_for(&self, branch: &str, step: &MergeView) -> bool {
        self.layers.contains(Layer::BranchMerge)
            && self.state.session.as_deref() == Some(branch)
            && self.merge.as_ref() == Some(step)
    }

    /// Has the model write the summary and puts it in the field.
    fn draft_branch_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(branch), Some(found)) = (
            self.state.project.clone(),
            self.state.session.clone(),
            self.merge.as_ref().and_then(MergeView::branch).cloned(),
        ) else {
            return;
        };
        let step = MergeView::Drafting(found.clone());
        self.merge = Some(step.clone());
        self.merge_error = None;
        let reply = self
            .backend
            .draft_branch_summary(project, branch.clone(), found.guard.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.merge_is_for(&branch, &step) {
                    return;
                }
                match result {
                    Ok(summary) => this
                        .merge_summary
                        .update(cx, |input, cx| input.set_value(summary, window, cx)),
                    // The field stays, to be written by hand.
                    Err(error) => this.merge_error = Some(error),
                }
                this.merge = Some(MergeView::Reviewing(found));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Adds the summary to the conversation the branch came from, and
    /// goes there.
    fn merge_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(branch), Some(MergeView::Reviewing(found))) = (
            self.state.project.clone(),
            self.state.session.clone(),
            self.merge.clone(),
        ) else {
            return;
        };
        let summary = self.merge_summary.read(cx).value().trim().to_owned();
        if summary.is_empty() {
            return;
        }
        let step = MergeView::Merging(found.clone());
        self.merge = Some(step.clone());
        self.merge_error = None;
        let reply = self.backend.merge_branch(
            project.clone(),
            branch.clone(),
            found.guard.clone(),
            summary,
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.merge_is_for(&branch, &step) {
                    return;
                }
                match result {
                    Ok(source) => {
                        this.close_branch_merge(window, cx);
                        this.open_session(project.clone(), source, window, cx);
                        this.refresh_lists(window, cx);
                        this.notice = Some(
                            tr("The branch's summary was added to the conversation it came from.")
                                .into(),
                        );
                    }
                    Err(error) => {
                        this.merge_error = Some(error);
                        this.merge = Some(MergeView::Reviewing(found.clone()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn close_branch_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::BranchMerge) {
            self.merge = None;
            self.merge_error = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_branch_merge(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let note = |text: String| div().text_sm().text_color(muted).child(text);
        // A click beside it does not throw a summary away: only Close and
        // Escape leave it.
        let mut card = surfaces::dialog(
            "branch-merge",
            DialogWidth::Regular,
            tr("Bring this branch back"),
        )
        .gap_3();
        if let Some(error) = &self.merge_error {
            card = card.child(error_banner(error, cx));
        }
        // The main action, after Close at the right end.
        let mut main = None;
        match &self.merge {
            None | Some(MergeView::Looking) => {
                card = card.child(note(tr("Looking at the branch…").into()));
            }
            Some(MergeView::Ready(found)) => {
                card = card.child(note(crate::i18n::fill(
                    tr("{} messages since it split off. Wisp writes a summary of them for you to read and change; it is then added to the conversation it came from, which reads it from then on."),
                    &[&found.new_messages],
                )));
                main =
                    Some(
                        Button::new("merge-draft")
                            .label(tr("Write the summary"))
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.draft_branch_summary(window, cx)
                            })),
                    );
            }
            Some(MergeView::Drafting(_)) => {
                card = card.child(note(tr("Writing the summary…").into()));
            }
            Some(MergeView::Reviewing(_)) | Some(MergeView::Merging(_)) => {
                let merging = matches!(self.merge, Some(MergeView::Merging(_)));
                let written = !self.merge_summary.read(cx).value().trim().is_empty();
                card = card
                    .child(note(
                        tr("Read and change it as you like; this is what the conversation it came from will have.")
                            .into(),
                    ))
                    .child(
                        div()
                            .h(px(300.))
                            .child(Input::new(&self.merge_summary).h_full()),
                    );
                card =
                    card.action(
                        Button::new("merge-redraft")
                            .label(tr("Write it again"))
                            .small()
                            .ghost()
                            .disabled(merging)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.draft_branch_summary(window, cx)
                            })),
                    );
                main = Some(
                    Button::new("merge-confirm")
                        .label(if merging {
                            tr("Working…")
                        } else {
                            tr("Bring it back")
                        })
                        .small()
                        .primary()
                        .disabled(merging || !written)
                        .on_click(cx.listener(|this, _, window, cx| this.merge_branch(window, cx))),
                );
            }
        }
        card.action(
            Button::new("merge-close")
                .label(tr("Close"))
                .small()
                .on_click(cx.listener(|this, _, window, cx| this.close_branch_merge(window, cx))),
        )
        .actions(main)
    }
}
