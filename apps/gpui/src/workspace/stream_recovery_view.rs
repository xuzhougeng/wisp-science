//! The bar above the composer while the running turn sends a failed model
//! request again, as the WebView's live recovery notice: what failed, when
//! it is tried again or that it is being rebuilt, and the provider's words.
//! Stop in the composer ends the turn meanwhile; the bar goes with the
//! next thing the model says or does, or with the turn's end.
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, StyledExt};

use super::{Workspace, surfaces};
use crate::{assets::icon, stream_recovery};

impl Workspace {
    pub(super) fn render_stream_recovery(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let snapshot = self.state.snapshot.as_ref().filter(|s| s.running)?;
        let (title, note, error) = stream_recovery::lines(snapshot.stream_recovery.as_ref()?);
        let muted = cx.theme().muted_foreground;
        Some(
            surfaces::bar("stream-recovery", cx)
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(icon("refresh").text_color(cx.theme().warning))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(title),
                        ),
                )
                .child(div().text_xs().text_color(muted).child(note))
                .children(
                    error.map(|error| div().text_xs().text_color(muted).truncate().child(error)),
                ),
        )
    }
}
