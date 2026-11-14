//! The side panel's Memos tab: the researcher's own notes for the project.
//! The model does not read them; a memo reaches a conversation only when it
//! is put into the message box and sent. The editor and the delete question
//! are layers inside the panel.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{ProjectMemo, ProjectMemoPatch};

use super::{
    Workspace,
    shell::{error_banner, glyph, icon_button},
};
use crate::{
    assets::icon,
    backend::{Reply, receive},
    layers::Layer,
    memos,
};

impl Workspace {
    /// Reads the project's memos. Only the answer for the project that is
    /// still open, to the latest request, is shown.
    pub(super) fn load_memos(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.memos_request += 1;
        let request = self.memos_request;
        let reply = self.backend.memos(project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.state.project.as_ref() != Some(&project) || this.memos_request != request {
                    return;
                }
                match result {
                    Ok(memos) => this.memos = Some(memos),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change to the memos and reads them again. `then` runs only
    /// if the same project is still open.
    fn change_memos<T: 'static>(
        &mut self,
        reply: Reply<T>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = true;
        self.error = None;
        let project = self.state.project.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if this.state.project != project {
                    return;
                }
                match result {
                    Ok(_) => {
                        then(this, window, cx);
                        this.load_memos(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn memo(&self, id: &str) -> Option<&ProjectMemo> {
        self.memos.as_ref()?.iter().find(|memo| memo.id == id)
    }

    /// Opens the editor on a new memo, or on `id` to change it.
    fn edit_memo(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let (title, body) = id
            .as_deref()
            .and_then(|id| self.memo(id))
            .map(|memo| (memo.title.clone(), memo.body.clone()))
            .unwrap_or_default();
        self.memo_target = id;
        self.layers.close(Layer::ConfirmDeleteMemo);
        self.layers.open(Layer::MemoEditor);
        self.memo_title
            .update(cx, |input, cx| input.set_value(title, window, cx));
        self.memo_body.update(cx, |input, cx| {
            input.set_value(body, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn save_memo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.layers.contains(Layer::MemoEditor) {
            return;
        }
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let title = self.memo_title.read(cx).value().to_string();
        let body = self.memo_body.read(cx).value().to_string();
        // Only on success: a refused memo keeps the editor and its text.
        let close = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.layers.close(Layer::MemoEditor);
            this.memo_target = None;
            this.focus_composer(window, cx);
        };
        match self.memo_target.clone() {
            Some(id) => {
                let patch = ProjectMemoPatch {
                    id,
                    title: Some(title),
                    body: Some(body),
                    pinned: None,
                };
                let reply = self.backend.update_memo(project, patch);
                self.change_memos(reply, close, window, cx);
            }
            None => {
                let reply = self.backend.add_memo(project, title, body);
                self.change_memos(reply, close, window, cx);
            }
        }
    }

    fn close_memo_layer(&mut self, layer: Layer, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(layer) {
            self.memo_target = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    fn pin_memo(&mut self, id: String, pinned: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let patch = ProjectMemoPatch {
            id,
            title: None,
            body: None,
            pinned: Some(pinned),
        };
        let reply = self.backend.update_memo(project, patch);
        self.change_memos(reply, |_, _, _| {}, window, cx);
    }

    fn ask_delete_memo(&mut self, id: String, cx: &mut Context<Self>) {
        self.memo_target = Some(id);
        self.layers.open(Layer::ConfirmDeleteMemo);
        cx.notify();
    }

    fn delete_memo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::ConfirmDeleteMemo) {
            return;
        }
        let (Some(project), Some(id)) = (self.state.project.clone(), self.memo_target.clone())
        else {
            return;
        };
        let reply = self.backend.delete_memo(project, id);
        self.change_memos(
            reply,
            |this, _, _| {
                this.layers.close(Layer::ConfirmDeleteMemo);
                this.memo_target = None;
            },
            window,
            cx,
        );
    }

    /// Puts a memo's text into the message box, after what is there.
    fn use_memo(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(body) = self.memo(id).map(|memo| memo.body.clone()) else {
            return;
        };
        let text = memos::insert(&self.composer.read(cx).value(), &body);
        self.state.save_draft(text.clone());
        self.composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn memo_editor(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let ready = memos::can_save(&self.memo_body.read(cx).value()) && !self.pending;
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .p_3()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(
                if self.memo_target.is_some() {
                    tr("Edit memo")
                } else {
                    tr("New memo")
                },
            ))
            .child(Input::new(&self.memo_title).small())
            .child(
                div()
                    .flex_1()
                    .min_h(px(160.))
                    .child(Input::new(&self.memo_body).h_full()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("A title is optional. The model does not read memos.")),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("memo-save")
                            .label(tr("Save memo"))
                            .small()
                            .primary()
                            .disabled(!ready)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_memo(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("memo-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_memo_layer(Layer::MemoEditor, window, cx)
                            })),
                    ),
            )
    }

    fn memo_card(&self, index: usize, memo: &ProjectMemo, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let asking = self.layers.contains(Layer::ConfirmDeleteMemo)
            && self.memo_target.as_deref() == Some(memo.id.as_str());
        let preview = memos::preview(memo);
        let mut card = div()
            .v_flex()
            .gap_1()
            .p_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(memos::heading(memo)),
                    )
                    .when(memo.pinned, |row| row.child(icon("pin").text_color(muted))),
            );
        if !preview.is_empty() {
            card = card.child(div().text_xs().text_color(muted).child(preview));
        }
        if asking {
            return card
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(tr("Delete this memo? It cannot be brought back.")),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_1()
                        .child(
                            Button::new("memo-delete-confirm")
                                .label(tr("Delete memo"))
                                .xsmall()
                                .danger()
                                .disabled(self.pending)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.delete_memo(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("memo-delete-cancel")
                                .label(tr("Cancel"))
                                .xsmall()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_memo_layer(Layer::ConfirmDeleteMemo, window, cx)
                                })),
                        ),
                );
        }
        let can_use = self.state.session.is_some();
        let (using, copying, pinning, editing, deleting) = (
            memo.id.clone(),
            memo.body.clone(),
            memo.id.clone(),
            memo.id.clone(),
            memo.id.clone(),
        );
        let pinned = memo.pinned;
        card.child(
            div()
                .h_flex()
                .gap_0p5()
                .child(
                    Button::new(("memo-use", index))
                        .icon(glyph("arrow-right"))
                        .label(tr("Use"))
                        .xsmall()
                        .ghost()
                        .tooltip(tr("Put this memo into the message box"))
                        .disabled(!can_use)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.use_memo(&using, window, cx)
                        })),
                )
                .child(div().flex_1())
                .child(
                    icon_button(("memo-copy", index), "copy", tr("Copy"))
                        .xsmall()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copying.clone()))
                        })),
                )
                .child(
                    icon_button(
                        ("memo-pin", index),
                        "pin",
                        if pinned {
                            tr("Unpin")
                        } else {
                            tr("Pin to the top")
                        },
                    )
                    .xsmall()
                    .disabled(self.pending)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pin_memo(pinning.clone(), !pinned, window, cx)
                    })),
                )
                .child(
                    icon_button(("memo-edit", index), "edit", tr("Edit"))
                        .xsmall()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_memo(Some(editing.clone()), window, cx)
                        })),
                )
                .child(
                    icon_button(("memo-delete", index), "trash", tr("Delete"))
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ask_delete_memo(deleting.clone(), cx)
                        })),
                ),
        )
    }

    pub(super) fn render_memos_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.layers.contains(Layer::MemoEditor) {
            let mut editor = div().v_flex().flex_1().min_h_0();
            if let Some(error) = &self.error {
                editor = editor.child(div().p_3().child(error_banner(error, cx)));
            }
            return editor.child(self.memo_editor(cx)).into_any_element();
        }
        let muted = cx.theme().muted_foreground;
        let mut list = div()
            .id("memos")
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .p_3()
            .overflow_y_scroll()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(muted)
                            .child(tr(
                                "Your own notes for this project. The model does not read them \
                         unless you put one into a message.",
                            )),
                    )
                    .child(
                        Button::new("memo-new")
                            .icon(glyph("note-plus"))
                            .label(tr("New memo"))
                            .small()
                            .flex_shrink_0()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.edit_memo(None, window, cx)),
                            ),
                    ),
            );
        if let Some(error) = &self.error {
            list = list.child(error_banner(error, cx));
        }
        match &self.memos {
            None => list = list.child(div().text_sm().text_color(muted).child(tr("Loading…"))),
            Some(memos) if memos.is_empty() => {
                list = list.child(div().text_sm().text_color(muted).child(tr(
                    "No memos yet. Keep a prompt you reuse, a server address or a reminder here.",
                )));
            }
            Some(memos) => {
                for (index, memo) in memos.iter().enumerate() {
                    list = list.child(self.memo_card(index, memo, cx));
                }
            }
        }
        list.into_any_element()
    }
}
