//! Sharing the open conversation as one HTML page, as the WebView's share
//! dialog does it: what was said is listed one message to a row, each
//! ticked, and only the ticked ones go into the page. The dialog is a layer
//! in the window's Escape stack.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{
    Workspace,
    shell::glyph,
    surfaces::{self, DialogWidth},
};
use crate::{
    backend::receive,
    layers::Layer,
    share::{self, ShareChoice, Sharing},
};

impl Workspace {
    /// Reads what was said in the open conversation and opens the dialog
    /// that picks which of it goes into the page, everything ticked.
    pub(super) fn share_conversation_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((project, session)), Some(title)) = (
            self.current_ids(),
            self.current_row().map(|row| row.title.clone()),
        ) else {
            return;
        };
        self.error = None;
        self.notice = None;
        let selection = self.state.selection();
        let reply = self.backend.share_rows(project, session.clone());
        cx.spawn_in(window, async move |this, cx| {
            let rows = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Only over the conversation it was asked for.
                if !this.state.current(&selection) {
                    return;
                }
                match rows {
                    Ok(rows) if rows.is_empty() => {
                        this.error = Some(tr("This conversation has nothing to share yet.").into());
                    }
                    Ok(rows) => {
                        this.sharing = Some(Sharing {
                            session,
                            title,
                            choice: ShareChoice::new(rows),
                            saving: false,
                        });
                        this.layers.open(Layer::ShareMessages);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn close_share(&mut self, cx: &mut Context<Self>) {
        self.layers.close(Layer::ShareMessages);
        self.sharing = None;
        cx.notify();
    }

    /// Saves the ticked messages as one page where the user chooses, light
    /// or dark as the window is. Dismissing the save dialog keeps the
    /// choice open, as the WebView does.
    fn export_shared_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sharing) = self.sharing.as_mut().filter(|sharing| !sharing.saving) else {
            return;
        };
        let rows = sharing.choice.chosen();
        let title = sharing.title.clone();
        let page = match share::html(&title, &rows, cx.theme().mode.is_dark()) {
            Ok(page) => page,
            Err(error) => {
                self.error = Some(error);
                return cx.notify();
            }
        };
        sharing.saving = true;
        let session = sharing.session.clone();
        self.error = None;
        let folder = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let place = cx.prompt_for_new_path(&folder, Some(&share::file_name(&title)));
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let Some(path) = place
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(None);
                };
                std::fs::write(&path, page).map_err(|error| {
                    crate::i18n::fill(tr("Could not save {}: {}"), &[&path.display(), &error])
                })?;
                Ok::<_, String>(Some(path))
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                let open = this
                    .sharing
                    .as_mut()
                    .filter(|sharing| sharing.session == session);
                match result {
                    Ok(Some(path)) => {
                        this.notice =
                            Some(crate::i18n::fill(tr("Saved to {}"), &[&path.display()]));
                        if open.is_some() {
                            this.close_share(cx);
                        }
                    }
                    Ok(None) => {
                        if let Some(sharing) = open {
                            sharing.saving = false;
                        }
                    }
                    Err(error) => {
                        if let Some(sharing) = open {
                            sharing.saving = false;
                        }
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The dialog: a row per message with its tick, Select all and Clear
    /// selection, how many are ticked, and Export HTML.
    pub(super) fn render_share(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let card = surfaces::dialog("share", DialogWidth::Regular, tr("Share as HTML"))
            .close_on_click_outside(cx.listener(|this, _, _, cx| this.close_share(cx)));
        let Some(sharing) = &self.sharing else {
            return card;
        };
        let choice = &sharing.choice;
        let mut list = div()
            .id("share-messages")
            .v_flex()
            .gap_0p5()
            .max_h(px(360.))
            .overflow_y_scroll();
        for (index, row) in choice.rows().iter().enumerate() {
            let picked = choice.is_picked(index);
            list = list.child(
                div()
                    .id(("share-message", index))
                    .h_flex()
                    .items_start()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .cursor_pointer()
                    .hover(|row| row.bg(cx.theme().list_hover))
                    .child(glyph(if picked { "check-square" } else { "circle" }).small())
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_xs().text_color(muted).child(share::speaker(row)))
                            .child(div().text_sm().line_clamp(2).child(share::preview(row))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(sharing) = &mut this.sharing {
                            sharing.choice.toggle(index);
                        }
                        cx.notify();
                    })),
            );
        }
        let set_all = |picked: bool| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, _, cx| {
                if let Some(sharing) = &mut this.sharing {
                    sharing.choice.set_all(picked);
                }
                cx.notify();
            })
        };
        let nothing = choice.count() == 0;
        let mut card = card
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Pick the messages to include.")),
            )
            .child(list);
        if nothing {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(tr("Select at least one message")),
            );
        }
        card.aside(
            Button::new("share-all")
                .label(tr("Select all"))
                .small()
                .ghost()
                .on_click(set_all(true)),
        )
        .aside(
            Button::new("share-none")
                .label(tr("Clear selection"))
                .small()
                .ghost()
                .on_click(set_all(false)),
        )
        .aside(div().text_xs().text_color(muted).child(choice.count_line()))
        .action(
            Button::new("share-cancel")
                .label(tr("Cancel"))
                .small()
                .ghost()
                .disabled(sharing.saving)
                .on_click(cx.listener(|this, _, _, cx| this.close_share(cx))),
        )
        .action(
            Button::new("share-export")
                .label(if sharing.saving {
                    tr("Exporting…")
                } else {
                    tr("Export HTML")
                })
                .small()
                .primary()
                .disabled(sharing.saving || nothing)
                .on_click(cx.listener(|this, _, window, cx| this.export_shared_page(window, cx))),
        )
    }
}
