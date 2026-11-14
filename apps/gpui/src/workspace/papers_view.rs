//! Paper evidence: a page listing the papers the open project is writing.
//! A paper has versions; a draft version holds figures, tables and claims,
//! each with the results and runs it rests on.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{
    Paper, PaperInput, PaperItem, PaperItemInput, PaperItemKind, PaperSourceChoice, PaperSourceKind,
};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, icon_button, nav_row},
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    papers,
    present::Tone,
};

/// The forms of the page; one is open at a time.
const FORMS: [Layer; 4] = [
    Layer::PaperForm,
    Layer::PaperItemForm,
    Layer::PaperSources,
    Layer::ConfirmRemovePaper,
];

impl Workspace {
    pub(super) fn open_papers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.project.is_none() {
            return;
        }
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::Papers);
        self.error = None;
        self.paper = None;
        self.load_papers(None, window, cx);
    }

    pub(super) fn close_papers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Papers) {
            self.paper = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    fn close_paper_forms(&mut self) {
        for form in FORMS {
            self.layers.close(form);
        }
    }

    /// Reads the project's papers, then shows `open`, or the paper that
    /// was showing, or the first.
    fn load_papers(&mut self, open: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let reply = self.backend.papers(project);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(papers) => {
                        let showing = this.paper.as_ref().map(|paper| paper.id.clone());
                        let next = open
                            .or(showing)
                            .filter(|id| papers.iter().any(|paper| &paper.id == id))
                            .or_else(|| papers.first().map(|paper| paper.id.clone()));
                        this.papers = papers;
                        match next {
                            Some(id) => this.show_paper(id, None, window, cx),
                            None => this.paper = None,
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Shows one paper at `version`, or at its newest. Only the answer to
    /// the latest request is shown.
    fn show_paper(
        &mut self,
        paper: String,
        version: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.paper_request += 1;
        let request = self.paper_request;
        let reply = self.backend.paper(project, paper, version);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.paper_request != request {
                    return;
                }
                match result {
                    Ok(paper) => this.paper = Some(paper),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change to the shown paper, then reads it again at the
    /// version that was showing. A refusal keeps the form open.
    fn change_paper(&mut self, reply: Reply<()>, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.close_paper_forms();
                        if let Some(paper) = &this.paper {
                            let (id, version) = (paper.id.clone(), paper.shown.clone());
                            this.show_paper(id, Some(version), window, cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the form for a new paper, or to rename the shown one.
    fn open_paper_form(&mut self, rename: bool, window: &mut Window, cx: &mut Context<Self>) {
        let values = match (&self.paper, rename) {
            (Some(paper), true) => [paper.title.clone(), paper.description.clone()],
            _ => Default::default(),
        };
        self.close_paper_forms();
        self.paper_renaming = rename;
        self.layers.open(Layer::PaperForm);
        self.error = None;
        for (field, value) in self.paper_fields.iter().zip(values) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.paper_fields[2].update(cx, |input, cx| input.set_value("", window, cx));
        self.paper_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_paper_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let [title, description, version] = self
            .paper_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        self.error = None;
        if let (true, Some(paper)) = (self.paper_renaming, &self.paper) {
            let id = paper.id.clone();
            let reply = self
                .backend
                .rename_paper(project, id.clone(), title, description);
            cx.spawn_in(window, async move |this, cx| {
                let result = receive(reply).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    match result {
                        Ok(()) => {
                            this.close_paper_forms();
                            this.load_papers(Some(id), window, cx);
                        }
                        Err(error) => this.error = Some(error),
                    }
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let reply = self.backend.create_paper(
            project,
            PaperInput {
                title,
                description,
                version,
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(paper) => {
                        this.close_paper_forms();
                        let id = paper.id.clone();
                        this.paper = Some(paper);
                        this.load_papers(Some(id), window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_paper(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(paper)) = (self.state.project.clone(), &self.paper) else {
            return;
        };
        let reply = self.backend.delete_paper(project, paper.id.clone());
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.close_paper_forms();
                match result {
                    Ok(()) => {
                        this.paper = None;
                        this.load_papers(None, window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the form on a new item, or on `item` to change it.
    fn open_paper_item_form(
        &mut self,
        item: Option<&PaperItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_paper_forms();
        self.paper_item_editing = Some(item.map(|item| item.id.clone()).unwrap_or_default());
        self.paper_item_kind = item.map_or(PaperItemKind::Figure, |item| item.kind);
        let values = item
            .map(|item| [item.title.clone(), item.note.clone()])
            .unwrap_or_default();
        self.layers.open(Layer::PaperItemForm);
        self.error = None;
        for (field, value) in self.paper_item_fields.iter().zip(values) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.paper_item_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_paper_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(paper), Some(id)) = (
            self.state.project.clone(),
            &self.paper,
            self.paper_item_editing.clone(),
        ) else {
            return;
        };
        let [title, note] = self
            .paper_item_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        let reply = self.backend.save_paper_item(
            project,
            paper.shown.clone(),
            PaperItemInput {
                id,
                kind: self.paper_item_kind,
                title,
                note,
            },
        );
        self.change_paper(reply, window, cx);
    }

    /// Opens the list of results and runs to choose evidence for `item`.
    fn open_paper_sources(&mut self, item: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close_paper_forms();
        self.paper_source_item = Some(item);
        self.layers.open(Layer::PaperSources);
        self.error = None;
        for field in &self.paper_source_fields {
            field.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.search_paper_sources(false, window, cx);
    }

    /// Reads the choices that match the search; `more` adds the next page.
    fn search_paper_sources(&mut self, more: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        if !more {
            self.paper_choices.clear();
            self.paper_choices_more = false;
        }
        let kind = self.paper_source_kind;
        let query = self.paper_source_fields[0].read(cx).value().to_string();
        let offset = self.paper_choices.len() as u32;
        self.paper_request += 1;
        let request = self.paper_request;
        let reply = self
            .backend
            .paper_source_choices(project, kind, query, offset);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.paper_request != request {
                    return;
                }
                match result {
                    Ok((choices, more)) => {
                        this.paper_choices.extend(choices);
                        this.paper_choices_more = more;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn bind_paper_source(
        &mut self,
        choice: &PaperSourceChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(project), Some(paper), Some(item)) = (
            self.state.project.clone(),
            &self.paper,
            self.paper_source_item.clone(),
        ) else {
            return;
        };
        let purpose = self.paper_source_fields[1].read(cx).value().to_string();
        let reply = self.backend.bind_paper_source(
            project,
            paper.shown.clone(),
            item,
            choice.kind,
            choice.id.clone(),
            purpose,
        );
        self.change_paper(reply, window, cx);
    }

    fn form_frame(&self, title: &'static str, cx: &App) -> Div {
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(620.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
    }

    fn form_actions(&self, save: Button, cx: &mut Context<Self>) -> Div {
        div().h_flex().gap_2().pt_1().child(save).child(
            Button::new("paper-form-cancel")
                .label(tr("Cancel"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.close_paper_forms();
                    cx.notify();
                })),
        )
    }

    fn paper_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let mut form = self
            .form_frame(
                if self.paper_renaming {
                    tr("Rename the paper")
                } else {
                    tr("New paper")
                },
                cx,
            )
            .child(label(tr("Title")))
            .child(Input::new(&self.paper_fields[0]).small())
            .child(label(tr("Description, if any")))
            .child(Input::new(&self.paper_fields[1]).small());
        if !self.paper_renaming {
            form = form
                .child(label(tr(
                    "What its first version is called. Empty: Initial submission",
                )))
                .child(Input::new(&self.paper_fields[2]).small());
        }
        let save = Button::new("paper-form-save")
            .label(tr("Save"))
            .small()
            .primary()
            .on_click(cx.listener(|this, _, window, cx| this.save_paper_form(window, cx)));
        form.child(self.form_actions(save, cx))
    }

    fn paper_item_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let mut kinds = div().h_flex().gap_1();
        for kind in PaperItemKind::OFFERED {
            kinds = kinds.child(
                Button::new(SharedString::from(format!("paper-item-kind-{kind:?}")))
                    .label(papers::kind_label(kind))
                    .small()
                    .selected(self.paper_item_kind == kind)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.paper_item_kind = kind;
                        cx.notify();
                    })),
            );
        }
        let adding = self.paper_item_editing.as_deref() == Some("");
        let save = Button::new("paper-item-save")
            .label(tr("Save"))
            .small()
            .primary()
            .on_click(cx.listener(|this, _, window, cx| this.save_paper_item(window, cx)));
        self.form_frame(
            if adding {
                tr("Add a figure, table or claim")
            } else {
                tr("Change this item")
            },
            cx,
        )
        .child(kinds)
        .child(label(tr("What it is called in the paper")))
        .child(Input::new(&self.paper_item_fields[0]).small())
        .child(label(tr("What it shows, if you want to note it")))
        .child(
            div()
                .h(px(96.))
                .child(Input::new(&self.paper_item_fields[1]).h_full()),
        )
        .child(self.form_actions(save, cx))
    }

    fn paper_sources_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let hover = cx.theme().list_hover;
        let mut kinds = div().h_flex().gap_1();
        for kind in [PaperSourceKind::Result, PaperSourceKind::Run] {
            kinds = kinds.child(
                Button::new(SharedString::from(format!("paper-source-kind-{kind:?}")))
                    .label(match kind {
                        PaperSourceKind::Result => tr("Project results"),
                        _ => tr("Runs"),
                    })
                    .small()
                    .selected(self.paper_source_kind == kind)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.paper_source_kind = kind;
                        this.search_paper_sources(false, window, cx);
                    })),
            );
        }
        let mut list = div()
            .id("paper-source-list")
            .v_flex()
            .max_h(px(260.))
            .overflow_y_scroll();
        if self.paper_choices.is_empty() {
            list = list.child(div().py_2().text_xs().text_color(muted).child(tr(
                "Nothing matches. A file has to be a registered result of the project \
                 before it can be chosen.",
            )));
        }
        for (index, choice) in self.paper_choices.iter().enumerate() {
            let picked = choice.clone();
            list = list.child(
                div()
                    .id(("paper-source", index))
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .text_sm()
                    .child(papers::choice_line(choice))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.bind_paper_source(&picked, window, cx)
                    })),
            );
        }
        let mut form = self
            .form_frame(tr("Add evidence"), cx)
            .child(div().text_xs().text_color(muted).child(tr(
                "Say why it supports the item, then choose a result or a run. A result is \
                 kept at the version it has now; later changes to the file do not alter it.",
            )))
            .child(Input::new(&self.paper_source_fields[1]).small())
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(kinds)
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.paper_source_fields[0]).small()),
                    )
                    .child(
                        Button::new("paper-source-search")
                            .label(tr("Search"))
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.search_paper_sources(false, window, cx)
                            })),
                    ),
            )
            .child(list);
        if self.paper_choices_more {
            form = form.child(
                div().h_flex().child(
                    Button::new("paper-source-more")
                        .label(tr("Show more"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search_paper_sources(true, window, cx)
                        })),
                ),
            );
        }
        form.child(
            div().h_flex().pt_1().child(
                Button::new("paper-source-cancel")
                    .label(tr("Cancel"))
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_paper_forms();
                        cx.notify();
                    })),
            ),
        )
    }

    fn paper_item_row(&self, paper: &Paper, item: &PaperItem, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let version = paper.shown.clone();
        let mut words = div().v_flex().flex_1().min_w_0().gap_1().child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(item.title.clone()),
        );
        if !item.note.is_empty() {
            words = words.child(div().text_xs().text_color(muted).child(item.note.clone()));
        }
        if item.sources.is_empty() {
            words = words.child(chip(tr("No source yet"), Tone::Danger, cx));
        }
        for source in &item.sources {
            let mut line = div()
                .h_flex()
                .gap_2()
                .child(div().w(px(64.)).child(chip(
                    papers::source_kind_label(source.kind),
                    Tone::Active,
                    cx,
                )))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .child(papers::source_line(source)),
                );
            if paper.editable {
                let (binding, version) = (source.id.clone(), version.clone());
                line = line.child(
                    icon_button(
                        SharedString::from(format!("paper-unbind-{}", source.id)),
                        "close",
                        "Remove this evidence",
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let Some(project) = this.state.project.clone() else {
                            return;
                        };
                        let reply = this.backend.unbind_paper_source(
                            project,
                            version.clone(),
                            binding.clone(),
                        );
                        this.change_paper(reply, window, cx);
                    })),
                );
            }
            words = words.child(line);
        }
        let mut row = div()
            .h_flex()
            .items_start()
            .gap_3()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().w(px(84.)).flex_shrink_0().child(chip(
                papers::kind_label(item.kind),
                Tone::Neutral,
                cx,
            )))
            .child(words);
        if paper.editable {
            let editing = item.clone();
            let (source_item, removed) = (item.id.clone(), item.id.clone());
            row = row.child(
                div()
                    .h_flex()
                    .gap_1()
                    .flex_shrink_0()
                    .child(
                        Button::new(SharedString::from(format!("paper-evidence-{}", item.id)))
                            .label(tr("Add evidence"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_paper_sources(source_item.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("paper-item-edit-{}", item.id)))
                            .label(tr("Change"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_paper_item_form(Some(&editing), window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("paper-item-remove-{}", item.id)))
                            .label(tr("Remove"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let Some(project) = this.state.project.clone() else {
                                    return;
                                };
                                let reply = this.backend.delete_paper_item(
                                    project,
                                    version.clone(),
                                    removed.clone(),
                                );
                                this.change_paper(reply, window, cx);
                            })),
                    ),
            );
        }
        row
    }

    fn paper_body(&self, paper: &Paper, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let asking = self.layers.contains(Layer::ConfirmRemovePaper);
        let mut actions = div().h_flex().gap_1().flex_shrink_0();
        if asking {
            actions = actions
                .child(
                    Button::new("paper-remove-yes")
                        .label(tr("Remove"))
                        .small()
                        .danger()
                        .on_click(cx.listener(|this, _, window, cx| this.remove_paper(window, cx))),
                )
                .child(
                    Button::new("paper-remove-no")
                        .label(tr("Keep"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmRemovePaper);
                            cx.notify();
                        })),
                );
        } else {
            actions = actions
                .child(
                    Button::new("paper-rename")
                        .label(tr("Rename"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_paper_form(true, window, cx)
                        })),
                )
                .child(
                    Button::new("paper-remove")
                        .label(tr("Remove"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_paper_forms();
                            this.layers.open(Layer::ConfirmRemovePaper);
                            cx.notify();
                        })),
                );
        }
        let mut heading = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(div().text_xl().child(paper.title.clone()));
        if !paper.description.is_empty() {
            heading = heading.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(paper.description.clone()),
            );
        }
        let mut versions = div().h_flex().flex_wrap().gap_1();
        for version in &paper.versions {
            let (id, target) = (paper.id.clone(), version.id.clone());
            versions = versions.child(
                Button::new(SharedString::from(format!("paper-version-{}", version.id)))
                    .label(papers::version_label(version))
                    .small()
                    .selected(version.id == paper.shown)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_paper_forms();
                        this.show_paper(id.clone(), Some(target.clone()), window, cx)
                    })),
            );
        }
        let mut body = div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .items_start()
                    .gap_4()
                    .child(heading)
                    .child(actions),
            )
            .child(versions)
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(papers::summary(paper)),
            );
        if !paper.editable && !paper.shown.is_empty() {
            body = body.child(div().text_xs().text_color(muted).child(tr(
                "This version is finalized: it shows the evidence it was locked with and \
                 cannot be changed here.",
            )));
        }
        for item in &paper.items {
            body = body.child(self.paper_item_row(paper, item, cx));
        }
        if !paper.loose.is_empty() {
            let mut loose = div().v_flex().gap_1().pt_2().child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Evidence kept with the version as a whole")),
            );
            for source in &paper.loose {
                loose = loose.child(div().text_xs().child(papers::source_line(source)));
            }
            body = body.child(loose);
        }
        if paper.editable && !self.layers.contains(Layer::PaperItemForm) {
            body = body.child(
                div().pt_2().h_flex().child(
                    Button::new("paper-item-add")
                        .icon(glyph("plus"))
                        .label(tr("Add a figure, table or claim"))
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_paper_item_form(None, window, cx)
                        })),
                ),
            );
        }
        body
    }

    pub(super) fn render_papers(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let project_name = self
            .state
            .project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .map(|project| project.name.clone())
            .unwrap_or_default();
        let showing = self.paper.as_ref().map(|paper| paper.id.as_str());
        let mut side = div()
            .id("papers-side")
            .v_flex()
            .w(px(260.))
            .flex_shrink_0()
            .h_full()
            .p_2()
            .gap_1()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .overflow_y_scroll()
            .child(
                nav_row("papers-back", "arrow-left", tr("Back"), false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_papers(window, cx))),
            )
            .child(
                div()
                    .px_2()
                    .pt_3()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("Paper evidence")),
            )
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(project_name),
            )
            .child(div().px_2().pb_2().text_xs().text_color(muted).child(tr(
                "The papers this project is writing: each figure, table and claim kept \
                 together with the results and runs it rests on.",
            )))
            .child(
                div().px_1().pb_2().child(
                    Button::new("paper-new")
                        .icon(glyph("plus"))
                        .label(tr("New paper"))
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_paper_form(false, window, cx)
                        })),
                ),
            );
        for paper in &self.papers {
            let id = paper.id.clone();
            side = side.child(
                nav_row(
                    SharedString::from(format!("paper-{}", paper.id)),
                    "doc",
                    paper.title.clone(),
                    showing == Some(paper.id.as_str()),
                    true,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_paper_forms();
                    this.show_paper(id.clone(), None, window, cx)
                })),
            );
        }

        let mut body = div()
            .id("papers-body")
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_6()
            .gap_3()
            .overflow_y_scroll();
        if let Some(error) = &self.error {
            body = body.child(error_banner(error, cx));
        }
        if self.layers.contains(Layer::PaperForm) {
            body = body.child(self.paper_form(cx));
        }
        if self.layers.contains(Layer::PaperItemForm) {
            body = body.child(self.paper_item_form(cx));
        }
        if self.layers.contains(Layer::PaperSources) {
            body = body.child(self.paper_sources_form(cx));
        }
        body = match &self.paper {
            Some(paper) => body.child(self.paper_body(paper, cx)),
            None if self.papers.is_empty() && !self.layers.contains(Layer::PaperForm) => body
                .child(div().text_sm().text_color(muted).child(tr(
                    "No paper yet. Name your paper, then start with a figure: keep its \
                     result, the run behind it and the reason you chose it together.",
                ))),
            None => body,
        };
        div().h_flex().size_full().child(side).child(body)
    }
}
