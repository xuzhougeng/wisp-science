//! "Generated · N" under an answer: the files its turn recorded, folded
//! into one caption line, opened into their folders. An inline disclosure,
//! not a layer, so it is not on the Escape stack; Tab reaches each fold and
//! file, and Enter or Space presses it.
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, StyledExt, tooltip::Tooltip};

use super::{Workspace, surfaces};
use crate::{
    assets::icon,
    backend::receive,
    generated::{self, Line},
    i18n::{fill, tr},
};

/// The tallest the opened list grows before it scrolls.
const LIST_HEIGHT: f32 = 18.;

impl Workspace {
    /// Asks the host which files the open conversation's turns recorded,
    /// for the answers that close them. A host that cannot say (a remote
    /// one) shows none; the Artifacts tab still lists them.
    pub(super) fn load_reply_artifacts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        self.reply_artifacts_request += 1;
        let request = self.reply_artifacts_request;
        let reply = self.backend.reply_artifacts(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) || this.reply_artifacts_request != request {
                    return;
                }
                let Ok(artifacts) = result else {
                    return;
                };
                if artifacts == this.reply_artifacts {
                    return;
                }
                this.reply_artifacts = artifacts;
                this.place_generated();
                this.load_inline_images(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Works out again which answer shows which files, and has the list
    /// measure the rows whose files changed. What was open stays open.
    pub(super) fn place_generated(&mut self) {
        let placed = generated::by_answer(&self.rows, &self.reply_artifacts);
        let changed: Vec<usize> = placed
            .iter()
            .filter(|(index, reply)| self.generated.get(index) != Some(reply))
            .map(|(index, _)| *index)
            .chain(
                self.generated
                    .keys()
                    .filter(|index| !placed.contains_key(index))
                    .copied(),
            )
            .collect();
        self.generated = placed;
        for index in changed {
            if index < self.rows.len() {
                self.list.splice(index..index + 1, 1);
            }
        }
    }

    /// The pictures recorded as artifacts of the turn the answer at `index`
    /// closes, to be drawn under it.
    pub(super) fn recorded_pictures(&self, index: usize) -> Vec<String> {
        self.generated
            .get(&index)
            .map(generated::pictures)
            .unwrap_or_default()
    }

    /// Opens or shuts an answer's list (`folder` empty) or one of its
    /// folders, and has the list measure the answer again.
    fn toggle_generated(
        &mut self,
        key: String,
        folder: String,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let fold = (key, folder);
        if !self.generated_open.remove(&fold) {
            self.generated_open.insert(fold);
        }
        if index < self.rows.len() {
            self.list.splice(index..index + 1, 1);
        }
        cx.notify();
    }

    /// The answer's "Generated · N", when its turn recorded files.
    pub(super) fn generated_section(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let reply = self.generated.get(&index)?;
        let muted = cx.theme().muted_foreground;
        let key = reply.key.clone();
        let open = self.generated_open.contains(&(key.clone(), String::new()));
        let toggle = key.clone();
        let summary = Press {
            id: SharedString::from(format!("generated-{key}")).into(),
            content: div()
                .h_flex()
                .gap_1()
                .px_1()
                .text_xs()
                .text_color(muted)
                .child(
                    icon(if open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    })
                    .text_color(muted),
                )
                .child(fill(tr("Generated · {}"), &[&reply.files.len()])),
            tooltip: None,
            on_press: Some(Box::new(cx.listener(move |this, _, _, cx| {
                this.toggle_generated(toggle.clone(), String::new(), index, cx)
            }))),
        };
        let mut section = div()
            .v_flex()
            .gap_1()
            .pt_1()
            .child(div().h_flex().child(summary));
        if !open {
            return Some(section.into_any_element());
        }
        let folds = &self.generated_open;
        let tree = generated::tree(&reply.files);
        let is_open = |path: &str| folds.contains(&(key.clone(), path.to_owned()));
        let mut list = surfaces::card(SharedString::from(format!("generated-list-{key}")), cx)
            .v_flex()
            .w_full()
            .max_w(px(640.))
            .max_h(rems(LIST_HEIGHT))
            .overflow_y_scroll()
            .p_1();
        for line in generated::lines(&tree, &is_open) {
            list = list.child(match line {
                Line::Folder {
                    path,
                    name,
                    count,
                    depth,
                    open,
                } => {
                    let (toggle, folder) = (key.clone(), path.clone());
                    Press {
                        id: SharedString::from(format!("generated-{key}-{path}")).into(),
                        content: line_box(depth)
                            .child(
                                icon(if open {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                })
                                .text_color(muted),
                            )
                            .child(icon("folder").text_color(muted))
                            .child(div().flex_1().min_w_0().truncate().text_sm().child(name))
                            .child(div().text_xs().text_color(muted).child(count.to_string())),
                        tooltip: None,
                        on_press: Some(Box::new(cx.listener(move |this, _, _, cx| {
                            this.toggle_generated(toggle.clone(), folder.clone(), index, cx)
                        }))),
                    }
                    .into_any_element()
                }
                Line::File { index: at, depth } => {
                    let file = &reply.files[at];
                    let picture = file.path.as_deref().is_some_and(crate::favorites::is_image);
                    let usable = !file.superseded && file.path.is_some();
                    let mut content = line_box(depth)
                        // In line with the names of the folders beside it.
                        .child(div().size_4().flex_none())
                        .child(icon(if picture { "image" } else { "doc" }).text_color(muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .when(!usable, |name| name.text_color(muted))
                                .child(file.name.clone()),
                        );
                    if file.superseded {
                        // A later answer wrote it again, as the WebView says.
                        content =
                            content.child(div().text_xs().text_color(muted).child(tr("Updated")));
                    }
                    let id = SharedString::from(format!("generated-{key}-file-{at}"));
                    match file.path.clone().filter(|_| usable) {
                        Some(path) => Press {
                            id: id.into(),
                            content,
                            tooltip: Some(path.clone().into()),
                            on_press: Some(Box::new(cx.listener(move |this, _, window, cx| {
                                this.reveal_file(path.clone(), window, cx)
                            }))),
                        }
                        .into_any_element(),
                        None => content.id(id).into_any_element(),
                    }
                }
            });
        }
        section = section.child(list);
        Some(section.into_any_element())
    }
}

/// A line of the opened list, indented by how deep it is.
fn line_box(depth: usize) -> Div {
    div()
        .h_flex()
        .gap_2()
        .py_1()
        .pr_2()
        .pl(rems(0.5 + depth as f32))
}

/// A line that is pressed with the pointer, or reached with Tab and pressed
/// with Enter or Space, as `surfaces::switch` is: a fold of the list or a
/// file in it, or a fold of a card in the transcript. Without `on_press` it
/// is only shown.
#[derive(IntoElement)]
pub(super) struct Press {
    pub(super) id: ElementId,
    pub(super) content: Div,
    /// The file's place in the project, shown when the pointer rests on it.
    pub(super) tooltip: Option<SharedString>,
    pub(super) on_press: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl RenderOnce for Press {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let focused = focus.is_focused(window);
        let hover = cx.theme().list_hover;
        let ring = if focused {
            cx.theme().ring
        } else {
            cx.theme().transparent
        };
        self.content
            .id(self.id)
            .rounded(cx.theme().radius)
            // A ring for the keyboard's focus; the pointer does not focus it.
            .border_1()
            .border_color(ring)
            .when_some(self.tooltip, |this, tooltip| {
                this.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            })
            .when_some(self.on_press, |this, on_press| {
                this.track_focus(&focus.tab_stop(true))
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(on_press)
            })
    }
}
