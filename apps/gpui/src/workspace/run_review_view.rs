//! The dialog that looks through what a finished run left on its SSH host,
//! one folder at a time: what is chosen is brought into the project or
//! deleted there, or the whole workspace goes.
use crate::i18n::{fill, tr};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{
    Workspace,
    shell::{error_banner, glyph, icon_button},
    surfaces::{self, DialogWidth},
};
use crate::{
    assets::icon,
    backend::receive,
    layers::Layer,
    run_review::{self, Deletion, RunReview},
};

impl Workspace {
    /// Opens the dialog on the top of `run`'s workspace.
    pub(super) fn open_run_review(
        &mut self,
        run: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_review = Some(RunReview::new(run, title));
        self.run_review_filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.layers.open(Layer::RunReview);
        self.list_run_review(false, window, cx);
    }

    fn close_run_review(&mut self, cx: &mut Context<Self>) {
        self.layers.close(Layer::RunReview);
        self.run_review = None;
        cx.notify();
    }

    /// Lists the folder shown from its start, or, with `more`, the entries
    /// after those listed.
    pub(super) fn list_run_review(
        &mut self,
        more: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let filter = self.run_review_filter.read(cx).value().to_string();
        let Some(review) = &mut self.run_review else {
            return;
        };
        review.request += 1;
        review.busy = true;
        review.problem = None;
        let (request, run) = (review.request, review.run.clone());
        let offset = if more { review.entries.len() } else { 0 };
        let reply =
            self.backend
                .run_workspace(project, run.clone(), review.path.clone(), filter, offset);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                let Some(review) = &mut this.run_review else {
                    return;
                };
                // Only the latest listing of the run still open.
                if review.run != run || review.request != request {
                    return;
                }
                review.busy = false;
                match result {
                    Ok(listing) => {
                        if !more {
                            review.entries.clear();
                        }
                        review.entries.extend(listing.entries);
                        review.truncated = listing.truncated;
                    }
                    Err(error) => review.problem = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn go_run_review(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(review) = &mut self.run_review {
            review.go(path);
        }
        self.list_run_review(false, window, cx);
    }

    /// Brings what is chosen into the project, as the conversation's
    /// results.
    fn bring_run_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let Some(review) = &mut self.run_review else {
            return;
        };
        let (files, folders) = review.chosen();
        if review.busy || (files.is_empty() && folders.is_empty()) {
            return;
        }
        review.busy = true;
        review.problem = None;
        review.note = None;
        let run = review.run.clone();
        let reply = self
            .backend
            .download_run_files(project, run.clone(), files, folders);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(review) = &mut this.run_review
                    && review.run == run
                {
                    review.busy = false;
                    match result {
                        Ok(count) => {
                            review.picked.clear();
                            review.note = Some(fill(tr("Brought {} into the project."), &[&count]));
                        }
                        Err(error) => review.problem = Some(error),
                    }
                }
                // The conversation's results list what came.
                this.reload_artifacts(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn ask_run_review_delete(&mut self, deletion: Deletion, cx: &mut Context<Self>) {
        if let Some(review) = &mut self.run_review {
            review.asking = Some(deletion);
            self.layers.open(Layer::ConfirmRunFilesDelete);
        }
        cx.notify();
    }

    /// Deletes what was asked about: the chosen entries, or the whole
    /// workspace with whatever was never brought back.
    fn delete_run_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let Some(review) = &mut self.run_review else {
            return;
        };
        let Some(deletion) = review.asking else {
            return;
        };
        if review.busy {
            return;
        }
        review.busy = true;
        review.problem = None;
        review.note = None;
        let run = review.run.clone();
        let reply = match deletion {
            Deletion::Chosen => self.backend.delete_run_files(
                project,
                run.clone(),
                review.picked.keys().cloned().collect(),
            ),
            Deletion::Whole => self.backend.clean_run(project, run.clone(), true),
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.layers.close(Layer::ConfirmRunFilesDelete);
                let Some(review) = &mut this.run_review else {
                    return;
                };
                if review.run != run {
                    return;
                }
                review.busy = false;
                review.asking = None;
                match (result, deletion) {
                    (Ok(()), Deletion::Chosen) => {
                        review.picked.clear();
                        review.note = Some(tr("Deleted there.").into());
                        this.list_run_review(false, window, cx);
                    }
                    (Ok(()), Deletion::Whole) => {
                        this.close_run_review(cx);
                        this.notice = Some(tr("The run's workspace is cleaned.").into());
                        // The run's new state arrives with the next snapshot.
                        this.start_poll(window, cx);
                    }
                    (Err(error), _) => review.problem = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// One entry of the folder shown: chosen or not, and a folder opens.
    fn run_review_row(
        &self,
        index: usize,
        entry: &wisp_dto::WorkspaceEntry,
        picked: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let folder = run_review::is_folder(entry);
        let mut name = div()
            .id(("review-name", index))
            .h_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(
                icon(if folder {
                    "folder"
                } else if crate::favorites::is_image(&entry.path) {
                    "image"
                } else {
                    "doc"
                })
                .text_color(muted),
            )
            .child(
                div()
                    .truncate()
                    .text_sm()
                    .child(run_review::name(&entry.path).to_owned()),
            );
        if folder {
            let path = entry.path.clone();
            name = name
                .cursor_pointer()
                .hover(|style| style.text_color(cx.theme().primary))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.go_run_review(path.clone(), window, cx)
                }));
        }
        let chosen = entry.clone();
        div()
            .h_flex()
            .gap_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new(("review-pick", index))
                    .icon(glyph(if picked { "check-square" } else { "circle" }))
                    .xsmall()
                    .ghost()
                    .tooltip(if picked {
                        tr("Leave it")
                    } else {
                        tr("Choose it")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(review) = &mut this.run_review {
                            review.toggle(&chosen);
                        }
                        cx.notify();
                    })),
            )
            .child(name)
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(muted)
                    .child(run_review::detail(entry)),
            )
    }

    /// The question before something there is deleted.
    fn run_review_question(&self, review: &RunReview, cx: &mut Context<Self>) -> Div {
        let asked = match review.asking {
            Some(Deletion::Whole) => tr(
                "Delete everything the run left there, outputs never brought back included? \
                 Its logs are saved into the project first.",
            )
            .to_owned(),
            _ => fill(
                tr("Delete the {} chosen there for good?"),
                &[&review.picked.len()],
            ),
        };
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().danger)
            .child(div().text_sm().child(asked))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("review-delete-yes")
                            .label(tr("Delete"))
                            .small()
                            .danger()
                            .disabled(review.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.delete_run_review(window, cx)
                            })),
                    )
                    .child(
                        Button::new("review-delete-no")
                            .label(tr("Cancel"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmRunFilesDelete);
                                if let Some(review) = &mut this.run_review {
                                    review.asking = None;
                                }
                                cx.notify();
                            })),
                    ),
            )
    }

    pub(super) fn render_run_review(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(review) = &self.run_review else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        let above = run_review::parent(&review.path);
        let navigation = div()
            .h_flex()
            .gap_2()
            .child(
                icon_button("review-up", "arrow-up", tr("The folder above"))
                    .disabled(above.is_none() || review.busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(above) = above.clone() {
                            this.go_run_review(above, window, cx);
                        }
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_family("monospace")
                    .child(run_review::location(&review.path)),
            )
            .child(
                div()
                    .w(px(220.))
                    .child(super::surfaces::input(&self.run_review_filter, cx).small()),
            )
            .child(
                icon_button("review-reload", "refresh", tr("Read it again"))
                    .disabled(review.busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.list_run_review(false, window, cx)),
                    ),
            );
        let mut list = div()
            .id("review-entries")
            .v_flex()
            .min_h(px(120.))
            .max_h(px(360.))
            .overflow_y_scroll();
        if review.entries.is_empty() {
            list = list.child(
                div()
                    .py_3()
                    .text_sm()
                    .text_color(muted)
                    .child(if review.busy {
                        tr("Reading…")
                    } else {
                        tr("Nothing here.")
                    }),
            );
        }
        for (index, entry) in review.entries.iter().enumerate() {
            let picked = review.picked.contains_key(&entry.path);
            list = list.child(self.run_review_row(index, entry, picked, cx));
        }
        if review.truncated {
            list = list.child(
                div().pt_2().child(
                    Button::new("review-more")
                        .label(tr("More…"))
                        .xsmall()
                        .ghost()
                        .disabled(review.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.list_run_review(true, window, cx)
                        })),
                ),
            );
        }
        let none = review.picked.is_empty();
        let mut card = surfaces::dialog(
            "run-review",
            DialogWidth::Wide,
            tr("What the run left on its host"),
        )
        .close_on_click_outside(cx.listener(|this, _, _, cx| this.close_run_review(cx)))
        .aside(
            div()
                .text_xs()
                .text_color(muted)
                .child(fill(tr("{} chosen"), &[&review.picked.len()])),
        )
        .action(
            Button::new("review-clean")
                .label(tr("Delete the whole workspace…"))
                .small()
                .ghost()
                .disabled(review.busy)
                .on_click(
                    cx.listener(|this, _, _, cx| this.ask_run_review_delete(Deletion::Whole, cx)),
                ),
        )
        .action(
            Button::new("review-delete")
                .label(tr("Delete chosen…"))
                .small()
                .disabled(none || review.busy)
                .on_click(
                    cx.listener(|this, _, _, cx| this.ask_run_review_delete(Deletion::Chosen, cx)),
                ),
        )
        .action(
            Button::new("review-close")
                .label(tr("Close"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| this.close_run_review(cx))),
        )
        .action(
            Button::new("review-bring")
                .icon(glyph("download"))
                .label(tr("Bring into the project"))
                .small()
                .primary()
                .disabled(none || review.busy)
                .on_click(cx.listener(|this, _, window, cx| this.bring_run_files(window, cx))),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted)
                .child(review.title.clone()),
        )
        .child(div().text_xs().text_color(muted).child(tr(
            "Choose files and folders to bring into the project (a folder comes as one \
                 archive) or to delete there. Nothing is copied until you ask.",
        )))
        .child(navigation);
        if let Some(problem) = &review.problem {
            card = card.child(error_banner(problem, cx));
        }
        if let Some(note) = &review.note {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().success)
                    .child(note.clone()),
            );
        }
        card = card.child(list);
        if review.asking.is_some() && self.layers.contains(Layer::ConfirmRunFilesDelete) {
            card = card.child(self.run_review_question(review, cx));
        }
        card.into_any_element()
    }
}
