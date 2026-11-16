//! Paper evidence: a page listing the papers the open project is writing.
//! A paper has versions; a draft version holds figures, tables and claims,
//! each with the results, runs and conversation passages it rests on. A
//! finalized version's runs can be rerun in a clean folder, and its
//! evidence saved as a capsule.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{
    Paper, PaperAudience, PaperCheck, PaperFinding, PaperInput, PaperItem, PaperItemInput,
    PaperItemKind, PaperPolicy, PaperSource, PaperSourceChoice, PaperSourceKind,
};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, icon_button, nav_row},
    surfaces,
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    papers,
    present::Tone,
};

/// The forms of the page; one is open at a time.
const FORMS: [Layer; 5] = [
    Layer::PaperForm,
    Layer::PaperItemForm,
    Layer::PaperSources,
    Layer::ConfirmRemovePaper,
    Layer::PaperFinalize,
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
        self.paper_check = None;
        self.paper_waiving = None;
        self.paper_passage = None;
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
                // The first version's usual name, in the language in use.
                version: match version.trim() {
                    "" => tr("Initial submission").to_owned(),
                    typed => typed.to_owned(),
                },
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
            self.paper_passage = None;
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

    /// Shows a conversation message to choose a passage of; the passage is
    /// selected in its text, read-only.
    fn choose_paper_message(
        &mut self,
        choice: PaperSourceChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = choice
            .message
            .as_ref()
            .and_then(|message| message.text.clone())
            .unwrap_or_default();
        self.paper_passage_field
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.paper_passage = Some(choice);
        self.error = None;
        cx.notify();
    }

    /// Binds the passage selected in the shown message, or the whole
    /// preview when nothing is selected, as the WebView does.
    fn bind_paper_passage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self
            .paper_passage
            .as_ref()
            .and_then(|choice| choice.message.clone())
        else {
            return;
        };
        let (shown, selected) = self.paper_passage_field.update(cx, |input, cx| {
            let selected = input
                .selected_text_range(false, window, cx)
                .map(|selection| selection.range)
                .unwrap_or_default();
            (input.value().to_string(), selected)
        });
        match papers::passage_locator(&message, &shown, selected) {
            Ok(locator) => {
                let choice = PaperSourceChoice {
                    kind: PaperSourceKind::Passage,
                    id: locator,
                    name: String::new(),
                    detail: String::new(),
                    message: None,
                };
                self.bind_paper_source(&choice, window, cx);
            }
            Err(refused) => {
                self.error = Some(refused.to_owned());
                cx.notify();
            }
        }
    }

    /// Reruns `run` of the shown finalized version in a clean folder, as
    /// the WebView's "Verify in clean workspace" does without asking first,
    /// then shows the report.
    fn verify_paper_run(&mut self, run: String, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(paper)) = (self.state.project.clone(), &self.paper) else {
            return;
        };
        if self.paper_rerunning.is_some() {
            return;
        }
        let (id, version) = (paper.id.clone(), paper.shown.clone());
        let reply = self
            .backend
            .verify_paper_run(project, version.clone(), run.clone());
        self.paper_rerunning = Some(run);
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.paper_rerunning = None;
                if let Err(error) = result {
                    this.error = Some(error);
                }
                // A rerun that ran and failed is recorded too.
                if this.paper.as_ref().map(|paper| paper.shown.as_str()) == Some(&version) {
                    this.show_paper(id, Some(version), window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Asks where to save the shown finalized version's capsule, then
    /// writes it there; the attempt shows under the version either way.
    fn save_paper_capsule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(paper)) = (self.state.project.clone(), &self.paper) else {
            return;
        };
        if self.paper_capsule_saving {
            return;
        }
        let (id, version) = (paper.id.clone(), paper.shown.clone());
        let name = wisp_server::desktop::paper_capsule_file_name(&paper.title, &version);
        let folder = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let place = cx.prompt_for_new_path(&folder, Some(&name));
        let backend = self.backend.clone();
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = place.await else {
                // The dialog was dismissed: nothing is written.
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.paper_capsule_saving = true;
                cx.notify();
            });
            let result = receive(backend.build_paper_capsule(project, version.clone(), path)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.paper_capsule_saving = false;
                if let Err(error) = result {
                    this.error = Some(error);
                }
                if this.paper.as_ref().map(|paper| paper.shown.as_str()) == Some(&version) {
                    this.show_paper(id, Some(version), window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the readiness check for the shown draft version, starting
    /// from evidence kept to yourself.
    fn open_paper_finalize(&mut self, cx: &mut Context<Self>) {
        self.close_paper_forms();
        self.paper_policy = PaperPolicy::default();
        self.layers.open(Layer::PaperFinalize);
        self.error = None;
        cx.notify();
    }

    /// Changes what the version is checked against; the last check no
    /// longer holds.
    fn set_paper_policy(&mut self, policy: PaperPolicy, cx: &mut Context<Self>) {
        if self.paper_policy != policy {
            self.paper_policy = policy;
            self.paper_check = None;
            self.paper_waiving = None;
            self.layers.close(Layer::PaperWaiver);
        }
        cx.notify();
    }

    /// Checks the shown version; with `finalize`, locks it when nothing
    /// stands in the way and shows it locked.
    fn check_paper_version(&mut self, finalize: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(paper)) = (self.state.project.clone(), &self.paper) else {
            return;
        };
        if self.paper_checking {
            return;
        }
        let (id, version) = (paper.id.clone(), paper.shown.clone());
        let policy = self.paper_policy;
        let reply = if finalize {
            self.backend
                .finalize_paper(project, version.clone(), policy)
        } else {
            self.backend.check_paper(project, version.clone(), policy)
        };
        self.paper_checking = true;
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.paper_checking = false;
                let current = this.paper.as_ref().map(|paper| paper.shown.as_str());
                if current != Some(version.as_str()) || this.paper_policy != policy {
                    return;
                }
                match result {
                    Ok(check) if check.finalized => {
                        this.close_paper_forms();
                        this.show_paper(id, Some(version), window, cx);
                    }
                    Ok(check) => this.paper_check = Some(check),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_paper_waiver(&mut self, code: String, window: &mut Window, cx: &mut Context<Self>) {
        self.paper_waiving = Some(code);
        self.layers.open(Layer::PaperWaiver);
        self.paper_waiver_field.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Records why the finding may stand, then checks again so it shows
    /// as let stand.
    fn save_paper_waiver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(paper), Some(code)) = (
            self.state.project.clone(),
            &self.paper,
            self.paper_waiving.clone(),
        ) else {
            return;
        };
        let reason = self.paper_waiver_field.read(cx).value().to_string();
        let reply = self
            .backend
            .waive_paper_finding(project, paper.shown.clone(), code, reason);
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.layers.close(Layer::PaperWaiver);
                        this.paper_waiving = None;
                        this.check_paper_version(false, window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Starts a draft from the shown version, with its items and evidence,
    /// and shows it.
    fn new_paper_version(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(paper)) = (self.state.project.clone(), &self.paper) else {
            return;
        };
        let reply = self.backend.new_paper_version(
            project,
            paper.id.clone(),
            paper.shown.clone(),
            papers::next_version_label(paper),
        );
        self.close_paper_forms();
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(paper) => {
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
        cx.notify();
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
                .child(div().text_xs().text_color(muted).child(crate::i18n::fill(
                    tr("What its first version is called. Empty: {}"),
                    &[&tr("Initial submission")],
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
        for kind in [
            PaperSourceKind::Result,
            PaperSourceKind::Run,
            PaperSourceKind::Passage,
        ] {
            kinds = kinds.child(
                Button::new(SharedString::from(format!("paper-source-kind-{kind:?}")))
                    .label(match kind {
                        PaperSourceKind::Result => tr("Project results"),
                        PaperSourceKind::Passage => tr("Research conversations"),
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
            list = list.child(div().py_2().text_xs().text_color(muted).child(
                match self.paper_source_kind {
                    PaperSourceKind::Passage => tr(
                        "Nothing matches. Only what you and the assistant said in this \
                         project's own conversations can be chosen.",
                    ),
                    _ => tr(
                        "Nothing matches. A file has to be a registered result of the project \
                         before it can be chosen.",
                    ),
                },
            ));
        }
        let chosen = self.paper_passage.as_ref().map(|choice| choice.id.as_str());
        let active = cx.theme().list_active;
        for (index, choice) in self.paper_choices.iter().enumerate() {
            let picked = choice.clone();
            let message = choice.kind == PaperSourceKind::Passage;
            list = list.child(
                div()
                    .id(("paper-source", index))
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .when(chosen == Some(choice.id.as_str()), |row| row.bg(active))
                    .hover(move |style| style.bg(hover))
                    .text_sm()
                    .when(message, |row| row.truncate())
                    .child(papers::choice_line(choice))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if message {
                            this.choose_paper_message(picked.clone(), window, cx)
                        } else {
                            this.bind_paper_source(&picked, window, cx)
                        }
                    })),
            );
        }
        let mut form = self
            .form_frame(tr("Add evidence"), cx)
            .child(div().text_xs().text_color(muted).child(tr(
                "Say why it supports the item, then choose a result, a run or a passage of a \
                 conversation. What you choose is kept as it is now; later changes do not \
                 alter it.",
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
        if let Some(choice) = &self.paper_passage {
            form = form.child(
                div()
                    .v_flex()
                    .gap_1()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!(
                                "{} · {}",
                                choice.name,
                                papers::role_label(&choice.detail)
                            )),
                    )
                    .child(div().text_xs().text_color(muted).child(tr(
                        "Select a passage below, or continue to use the full preview (up to \
                         16 KiB).",
                    )))
                    .child(
                        div().h(px(180.)).child(
                            Input::new(&self.paper_passage_field)
                                .disabled(true)
                                .h_full(),
                        ),
                    )
                    .child(div().text_xs().text_color(muted).child(tr(
                        "The selected version or passage is saved; later edits do not update it.",
                    )))
                    .child(
                        div().h_flex().child(
                            Button::new("paper-passage-bind")
                                .label(tr("Add this passage"))
                                .small()
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.bind_paper_passage(window, cx)
                                })),
                        ),
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

    /// One finding of the check: what it is, and either the reason it was
    /// let stand or, when it may, a way to give one.
    fn paper_finding_row(
        &self,
        index: usize,
        finding: &PaperFinding,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut words = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_0p5()
            .child(div().text_sm().child(crate::i18n::host(&finding.message)))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(finding.code.clone()),
            );
        if let Some(reason) = &finding.waived {
            words = words.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(crate::i18n::fill(tr("Let stand: {}"), &[reason])),
            );
        }
        let mut row = div()
            .v_flex()
            .gap_1()
            .py_1()
            .child(div().h_flex().items_start().gap_2().child(words));
        let asking = self.paper_waiving.as_deref() == Some(finding.code.as_str())
            && self.layers.contains(Layer::PaperWaiver);
        if asking {
            row = row.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.paper_waiver_field).small()),
                    )
                    .child(
                        Button::new(("paper-waiver-save", index))
                            .label(tr("Save"))
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_paper_waiver(window, cx)
                            })),
                    )
                    .child(
                        Button::new(("paper-waiver-cancel", index))
                            .label(tr("Cancel"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::PaperWaiver);
                                this.paper_waiving = None;
                                cx.notify();
                            })),
                    ),
            );
        } else if finding.waivable && finding.waived.is_none() {
            let code = finding.code.clone();
            row = row.child(
                div().h_flex().child(
                    Button::new(("paper-waive", index))
                        .label(tr("Let it stand…"))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_paper_waiver(code.clone(), window, cx)
                        })),
                ),
            );
        }
        row
    }

    fn paper_findings(
        &self,
        title: &'static str,
        offset: usize,
        findings: &[PaperFinding],
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if findings.is_empty() {
            return None;
        }
        let mut section = div().v_flex().gap_1().pt_1().child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .child(crate::i18n::fill(title, &[&findings.len()])),
        );
        for (index, finding) in findings.iter().enumerate() {
            section = section.child(self.paper_finding_row(offset + index, finding, cx));
        }
        Some(section)
    }

    fn paper_check_result(&self, check: &PaperCheck, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let tone = if check.can_finalize {
            Tone::Success
        } else {
            Tone::Danger
        };
        let mut result = div()
            .v_flex()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(chip(papers::verdict(check), tone, cx))
                    .child(div().text_xs().text_color(muted).child(crate::i18n::fill(
                        tr("Evidence level: {}"),
                        &[&papers::level_label(check.level)],
                    ))),
            );
        let blockers = check.blockers.len();
        let warnings = check.warnings.len();
        for section in [
            self.paper_findings(tr("Stands in the way ({})"), 0, &check.blockers, cx),
            self.paper_findings(tr("Warnings ({})"), blockers, &check.warnings, cx),
            self.paper_findings(
                tr("Left out of the record ({})"),
                blockers + warnings,
                &check.omissions,
                cx,
            ),
        ]
        .into_iter()
        .flatten()
        {
            result = result.child(section);
        }
        result
    }

    /// The panel that checks the shown draft and finalizes it.
    fn paper_finalize_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let policy = self.paper_policy;
        let mut audiences = div().h_flex().gap_1();
        for audience in PaperAudience::ALL {
            audiences = audiences.child(
                Button::new(SharedString::from(format!("paper-audience-{audience:?}")))
                    .label(papers::audience_label(audience))
                    .small()
                    .selected(policy.audience == audience)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let policy = PaperPolicy {
                            audience,
                            ..this.paper_policy
                        };
                        this.set_paper_policy(policy, cx)
                    })),
            );
        }
        let confirm =
            |id: &'static str, text: &'static str, on: bool, change: fn(&mut PaperPolicy)| {
                Button::new(id)
                    .icon(glyph(if on { "check-square" } else { "square" }))
                    .label(text)
                    .small()
                    .ghost()
                    .selected(on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut policy = this.paper_policy;
                        change(&mut policy);
                        this.set_paper_policy(policy, cx)
                    }))
            };
        let mut confirms = div().v_flex().items_start().gap_1();
        if policy.audience == PaperAudience::Public {
            confirms = confirms
                .child(confirm(
                    "paper-personal-reviewed",
                    tr("I looked over the evidence for health and personal details"),
                    policy.personal_reviewed,
                    |policy| policy.personal_reviewed = !policy.personal_reviewed,
                ))
                .child(confirm(
                    "paper-rights-reviewed",
                    tr("I looked over licences and the right to pass it on"),
                    policy.rights_reviewed,
                    |policy| policy.rights_reviewed = !policy.rights_reviewed,
                ));
        } else {
            confirms = confirms.child(confirm(
                "paper-keep-restricted",
                tr("Copy restricted files into the record where allowed"),
                policy.keep_restricted,
                |policy| policy.keep_restricted = !policy.keep_restricted,
            ));
        }
        let ready = self
            .paper_check
            .as_ref()
            .is_some_and(|check| check.can_finalize);
        let mut actions = div()
            .h_flex()
            .gap_2()
            .pt_1()
            .child(
                Button::new("paper-check")
                    .icon(glyph("check"))
                    .label(tr("Check this version"))
                    .small()
                    .disabled(self.paper_checking)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.check_paper_version(false, window, cx)
                    })),
            )
            .child(
                Button::new("paper-finalize-confirm")
                    .icon(glyph("lock"))
                    .label(tr("Finalize and lock the evidence"))
                    .small()
                    .primary()
                    .disabled(self.paper_checking || !ready)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.check_paper_version(true, window, cx)
                    })),
            )
            .child(
                Button::new("paper-finalize-cancel")
                    .label(tr("Cancel"))
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_paper_forms();
                        cx.notify();
                    })),
            );
        if self.paper_checking {
            actions = actions.child(div().text_xs().text_color(muted).child(tr("Checking…")));
        }
        let mut form = self
            .form_frame(tr("Finalize this version"), cx)
            .child(div().text_xs().text_color(muted).child(tr(
                "Check the evidence first. Finalizing locks this version; later changes need a \
                 new version.",
            )))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Who the evidence may be shown to")),
            )
            .child(audiences)
            .child(confirms)
            .child(actions)
            .child(div().text_xs().text_color(muted).child(tr(
                "Finalizing keeps the evidence. Whether the results can be made again is a \
                 separate question that a passing check does not answer.",
            )));
        match &self.paper_check {
            Some(check) => form = form.child(self.paper_check_result(check, cx)),
            None if !self.paper_checking => {
                form = form.child(div().text_xs().text_color(muted).child(tr(
                    "Check this version to see what is missing, what to look at, and what \
                     the record will leave out.",
                )))
            }
            None => {}
        }
        form
    }

    /// On evidence of a finalized version that a run made: rerun that run
    /// in a clean folder.
    fn paper_verify_button(
        &self,
        paper: &Paper,
        source: &PaperSource,
        cx: &mut Context<Self>,
    ) -> Option<Button> {
        let run = source.run.clone()?;
        if !papers::shown_is_finalized(paper) {
            return None;
        }
        Some(
            Button::new(SharedString::from(format!("paper-verify-{}", source.id)))
                .label(tr("Verify in a clean folder"))
                .xsmall()
                .ghost()
                .disabled(self.paper_rerunning.is_some())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.verify_paper_run(run.clone(), window, cx)
                })),
        )
    }

    /// What clean reruns and capsules of the shown finalized version
    /// recorded.
    fn paper_records(&self, paper: &Paper, cx: &mut Context<Self>) -> Option<Div> {
        if paper.reruns.is_empty() && paper.capsules.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let mut records = div().v_flex().gap_2().pt_2();
        if !paper.reruns.is_empty() {
            let mut section = div().v_flex().gap_1().child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("Reproduction reports")),
            );
            for rerun in &paper.reruns {
                let mut entry = div()
                    .v_flex()
                    .gap_0p5()
                    .py_1()
                    .child(div().text_sm().child(papers::rerun_line(rerun)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("run:{}", rerun.run)),
                    );
                for output in &rerun.outputs {
                    entry = entry.child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(chip(
                                if output.passed {
                                    tr("Matches")
                                } else {
                                    tr("Differs")
                                },
                                if output.passed {
                                    Tone::Success
                                } else {
                                    Tone::Danger
                                },
                                cx,
                            ))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .child(papers::rerun_output_line(output)),
                            ),
                    );
                }
                if let Some(error) = &rerun.error {
                    entry = entry.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(crate::i18n::host(error)),
                    );
                }
                section = section.child(entry);
            }
            records = records.child(section);
        }
        if !paper.capsules.is_empty() {
            let mut section = div().v_flex().gap_1().child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("Capsule builds")),
            );
            for capsule in &paper.capsules {
                let mut entry = div()
                    .v_flex()
                    .gap_0p5()
                    .py_1()
                    .child(div().text_sm().child(papers::capsule_line(capsule)));
                if !capsule.path.is_empty() {
                    entry = entry.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .truncate()
                            .child(capsule.path.clone()),
                    );
                }
                if let Some(error) = &capsule.error {
                    entry = entry.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(crate::i18n::host(error)),
                    );
                }
                section = section.child(entry);
            }
            records = records.child(section);
        }
        Some(records)
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
            } else if let Some(verify) = self.paper_verify_button(paper, source, cx) {
                line = line.child(verify);
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
            if paper.editable && !self.layers.contains(Layer::PaperFinalize) {
                actions = actions.child(
                    Button::new("paper-finalize")
                        .icon(glyph("lock"))
                        .label(tr("Finalize…"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| this.open_paper_finalize(cx))),
                );
            }
            if papers::shown_is_finalized(paper) {
                actions = actions.child(
                    Button::new("paper-capsule")
                        .icon(glyph("download"))
                        .label(tr("Build capsule…"))
                        .small()
                        .ghost()
                        .disabled(self.paper_capsule_saving)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.save_paper_capsule(window, cx)),
                        ),
                );
            }
            if !paper.shown.is_empty() {
                actions = actions.child(
                    Button::new("paper-new-version")
                        .icon(glyph("note-plus"))
                        .label(tr("New version"))
                        .small()
                        .ghost()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.new_paper_version(window, cx)),
                        ),
                );
            }
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
            .child(super::surfaces::title(paper.title.clone()));
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
            if let Some(locked) = paper
                .versions
                .iter()
                .find(|version| version.id == paper.shown && !version.fingerprint.is_empty())
            {
                body = body.child(div().text_xs().text_color(muted).child(crate::i18n::fill(
                    tr("Locked under {}. New version starts a draft from it."),
                    &[&papers::fingerprint(&locked.fingerprint)],
                )));
            }
            if let Some(level) = paper.level {
                body = body.child(div().text_xs().text_color(muted).child(crate::i18n::fill(
                    tr("Evidence level: {}"),
                    &[&papers::level_label(level)],
                )));
            }
            if papers::shown_is_finalized(paper) {
                body = body.child(div().text_xs().text_color(muted).child(tr(
                    "Finalization preserves evidence. Reproduction is verified separately and \
                     is not implied by a successful check.",
                )));
            }
            if self.paper_rerunning.is_some() {
                body = body.child(div().text_xs().text_color(muted).child(tr(
                    "Rerunning in a clean folder with only the frozen inputs and code…",
                )));
            }
            if self.paper_capsule_saving {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(tr("Writing the capsule…")),
                );
            }
        }
        if self.layers.contains(Layer::PaperFinalize) && paper.editable {
            body = body.child(self.paper_finalize_form(cx));
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
                let mut line = div().h_flex().gap_2().child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .child(papers::source_line(source)),
                );
                if let Some(verify) = self.paper_verify_button(paper, source, cx) {
                    line = line.child(verify);
                }
                loose = loose.child(line);
            }
            body = body.child(loose);
        }
        if let Some(records) = self.paper_records(paper, cx) {
            body = body.child(records);
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
            .child(surfaces::title(tr("Paper evidence")).px_2().pt_3())
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
