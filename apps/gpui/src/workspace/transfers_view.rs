//! Files moving between machines for the open conversation: a tray above
//! the composer while they move, and the same meter on the run's card.
use crate::i18n::{fill, tr};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, StyledExt};

use super::{Workspace, shell::icon_button};
use crate::transfers;

/// One transfer's meter: where it is, which file, how much, and a bar when
/// the share done is known.
pub(super) fn meter(progress: &wisp_dto::RunProgress, cx: &App) -> Div {
    let shown = transfers::meter(progress);
    let muted = cx.theme().muted_foreground;
    let mut head = div()
        .h_flex()
        .gap_2()
        .text_xs()
        .child(div().font_semibold().child(shown.phase));
    if let Some(file) = shown.file {
        head = head.child(div().min_w_0().truncate().text_color(muted).child(file));
    }
    let track = div().h(px(4.)).w_full().rounded_full().bg(cx.theme().muted);
    let bar = match shown.done {
        Some(done) => track.child(
            div()
                .h_full()
                .w(relative(done))
                .rounded_full()
                .bg(cx.theme().primary),
        ),
        // How far it is cannot be told: a bar of the muted colour only.
        None => track,
    };
    div()
        .v_flex()
        .gap_1()
        .child(head)
        .child(div().text_xs().text_color(muted).child(shown.detail))
        .child(bar)
}

impl Workspace {
    /// The open conversation's transfers that show now, above the composer.
    pub(super) fn render_transfers(&self, cx: &mut Context<Self>) -> Option<Div> {
        let runs = &self.state.snapshot.as_ref()?.run_cards;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let shown = transfers::shown(runs, now);
        if shown.is_empty() {
            return None;
        }
        let folded = self.transfers_folded;
        let mut tray = div()
            .v_flex()
            .gap_2()
            .mx_4()
            .mb_2()
            .px_3()
            .py_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_semibold()
                            .child(fill(tr("Transfers ({})"), &[&shown.len()])),
                    )
                    .child(
                        icon_button(
                            "transfers-fold",
                            if folded { "chevron-down" } else { "chevron-up" },
                            if folded {
                                tr("Show the transfers")
                            } else {
                                tr("Fold the transfers")
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.transfers_folded = !this.transfers_folded;
                            cx.notify();
                        })),
                    ),
            );
        if !folded {
            for (run, progress) in &shown {
                tray = tray.child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(div().text_xs().truncate().child(run.title.clone()))
                        .child(meter(progress, cx)),
                );
            }
        }
        Some(tray)
    }
}
