//! The Notebook tab of the side panel: the code of the open conversation
//! as cells, with what each run returned.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, StyledExt};

use super::{Workspace, chip, shell::icon_button, surfaces};
use crate::{
    notebook,
    present::{self, Tone},
};

/// How much of a cell's output the panel shows before pointing at the
/// conversation for the rest.
const OUTPUT_LINES: usize = 14;
const OUTPUT_BYTES: usize = 2_400;

impl Workspace {
    /// Saves a cell's code to the favorites, or takes it out if it is there.
    fn star_cell(
        &mut self,
        language: String,
        source: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .saved
            .iter()
            .find(|item| item.kind == "code" && item.code == source)
            .map(|item| item.id.clone())
        {
            return self.unstar(id, window, cx);
        }
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let reply = self.backend.star_code(project, session, language, source);
        self.change_favorites(
            reply,
            |this, item, _, _| this.saved.insert(0, item),
            window,
            cx,
        );
    }

    pub(super) fn render_notebook_tab(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let cells = notebook::cells(&self.rows);
        let summary = notebook::summary(&cells);
        // With no cell the count is what the tab has to say.
        let mut list = div()
            .id("notebook-cells")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(if cells.is_empty() {
                surfaces::panel_empty(summary, cx)
            } else {
                surfaces::panel_note(summary, cx)
            });
        if self.before.is_some() {
            list = list.child(surfaces::panel_note(
                tr("Code in earlier messages is listed once they are loaded."),
                cx,
            ));
        }
        for cell in cells {
            let saved = self
                .saved
                .iter()
                .any(|item| item.kind == "code" && item.code == cell.source);
            let (copying, starring, language, row) = (
                cell.source.clone(),
                cell.source.clone(),
                cell.language.clone(),
                cell.row,
            );
            let mut block = surfaces::panel_entry(cx)
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(format!("[{}]", cell.number)),
                        )
                        .child(chip(cell.language.clone(), Tone::Neutral, cx))
                        .child(div().text_xs().text_color(muted).child(if cell.ran {
                            tr("run")
                        } else {
                            tr("shown, not run")
                        }))
                        .child(div().flex_1())
                        .child(
                            icon_button(
                                ("notebook-copy", cell.number),
                                "copy",
                                tr("Copy the code"),
                            )
                            .on_click(cx.listener(
                                move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copying.clone(),
                                    ))
                                },
                            )),
                        )
                        .child(
                            icon_button(
                                ("notebook-star", cell.number),
                                if saved { "star-filled" } else { "star" },
                                if saved {
                                    tr("Take it out of your favorites")
                                } else {
                                    tr("Save the code to your favorites")
                                },
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.star_cell(language.clone(), starring.clone(), window, cx)
                                },
                            )),
                        )
                        .child(
                            icon_button(
                                ("notebook-show", cell.number),
                                "arrow-right",
                                tr("Show it in the conversation"),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.list.scroll_to_reveal_item(row);
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .child(
                    surfaces::markdown(
                        SharedString::from(format!("notebook-source-{}", cell.number)),
                        present::fenced(&cell.source, &cell.language),
                        window,
                        cx,
                    )
                    .selectable(true),
                );
            let (output, cut) = notebook::output_preview(&cell.output, OUTPUT_LINES, OUTPUT_BYTES);
            if !output.is_empty() {
                block = block.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(output),
                );
            }
            if cut {
                block = block.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(tr("… the rest is in the conversation")),
                );
            }
            list = list.child(block);
        }
        list.into_any_element()
    }
}
