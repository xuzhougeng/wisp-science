//! The Notebook tab of the side panel: the code of the open conversation
//! as cells, with what each run returned.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, StyledExt, text::TextView};

use super::{Workspace, chip, shell::icon_button};
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
        let border = cx.theme().border;
        let cells = notebook::cells(&self.rows);
        let mut list = div()
            .id("notebook-cells")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .text_xs()
                    .text_color(muted)
                    .child(notebook::summary(&cells)),
            );
        if self.before.is_some() {
            list = list.child(div().px_3().pt_1().text_xs().text_color(muted).child(tr(
                "Code in earlier messages is listed once they are loaded.",
            )));
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
            let mut block = div()
                .v_flex()
                .gap_1()
                .p_3()
                .border_b_1()
                .border_color(border)
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
                            "run"
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
                    TextView::markdown(
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
