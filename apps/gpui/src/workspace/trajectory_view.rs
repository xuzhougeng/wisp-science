//! The trajectory of the open conversation: a dialog with the totals over
//! the whole conversation and, turn by turn, every step in the order it
//! happened. A step with more to show than its row opens in place.
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, StyledExt, button::Button};

use super::{
    Workspace,
    shell::{error_banner, glyph},
};
use crate::{
    backend::receive,
    layers::Layer,
    trajectory::{self, Tone},
};

impl Workspace {
    pub(super) fn open_trajectory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::Trajectory);
        self.trajectory = None;
        self.trajectory_error = None;
        self.trajectory_open.clear();
        let reply = self.backend.trajectory(project, session.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Closed, or another conversation, while this was read.
                if !this.layers.contains(Layer::Trajectory)
                    || this.state.session.as_deref() != Some(session.as_str())
                {
                    return;
                }
                match result {
                    Ok(found) => this.trajectory = Some(found),
                    Err(error) => this.trajectory_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Forgets what the dialog was showing, once its layer is gone.
    pub(super) fn clear_trajectory(&mut self) {
        self.trajectory = None;
        self.trajectory_error = None;
        self.trajectory_open.clear();
    }

    fn close_trajectory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Trajectory) {
            self.clear_trajectory();
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_trajectory(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let mut card = div()
            .id("trajectory")
            .v_flex()
            .gap_2()
            .w(px(760.))
            .max_w(relative(0.94))
            .max_h(relative(0.9))
            .overflow_y_scroll()
            .p_5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(border)
            .bg(cx.theme().popover)
            .shadow_lg()
            // A click inside is not a click on the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Trajectory"),
            );
        if let Some(error) = &self.trajectory_error {
            card = card.child(error_banner(error, cx));
        }
        match &self.trajectory {
            None if self.trajectory_error.is_none() => {
                card = card.child(div().text_sm().text_color(muted).child("Reading…"));
            }
            None => {}
            Some(found) => {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(trajectory::subtitle(found)),
                );
                if !found.turns.is_empty() {
                    card = card.child(self.trajectory_totals(found, cx));
                }
                let zone = chrono::Local;
                for (turn_at, turn) in found.turns.iter().enumerate() {
                    card = card.child(
                        div()
                            .pt_3()
                            .pb_1()
                            .border_b_1()
                            .border_color(border)
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(trajectory::turn_heading(turn, &zone)),
                    );
                    for (cell_at, cell) in turn.cells.iter().enumerate() {
                        card = card.child(self.trajectory_step(turn_at, cell_at, cell, cx));
                    }
                }
            }
        }
        card = card.child(
            div().h_flex().justify_end().pt_2().child(
                Button::new("trajectory-close")
                    .label("Close")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.close_trajectory(window, cx))),
            ),
        );
        div()
            .id("trajectory-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            // Nothing underneath is clickable while the dialog is open.
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(cx.theme().overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_trajectory(window, cx)),
            )
            .child(card)
    }

    fn trajectory_totals(
        &self,
        found: &wisp_dto::TrajectorySnapshotDto,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let mut row = div().h_flex().flex_wrap().gap_2().pt_1();
        for (label, value) in trajectory::totals(&found.stats) {
            row = row.child(
                div()
                    .v_flex()
                    .px_3()
                    .py_1p5()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(div().text_xs().text_color(muted).child(label))
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(value)),
            );
        }
        row
    }

    fn trajectory_step(
        &self,
        turn_at: usize,
        cell_at: usize,
        cell: &wisp_dto::TrajectoryCellDto,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let step = trajectory::step(cell);
        let detail = trajectory::detail(cell);
        let key = (turn_at, cell_at);
        let open = detail.is_some() && self.trajectory_open.contains(&key);
        let label_color = match step.tone {
            Tone::Plain => cx.theme().foreground,
            Tone::Failed => cx.theme().danger,
            Tone::Quiet => muted,
        };
        let mut row = div()
            .id(SharedString::from(format!(
                "trajectory-step-{turn_at}-{cell_at}"
            )))
            .h_flex()
            .items_start()
            .gap_2()
            .py_1()
            .px_1()
            .rounded(cx.theme().radius)
            .child(
                // The arrow keeps its place on rows that do not open, so the
                // labels line up.
                div()
                    .w(px(14.))
                    .pt_0p5()
                    .flex_shrink_0()
                    .children(detail.is_some().then(|| {
                        glyph(if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        })
                        .xsmall()
                        .text_color(muted)
                    })),
            )
            .child(
                div()
                    .w(px(84.))
                    .flex_shrink_0()
                    .text_xs()
                    .pt_0p5()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(label_color)
                    .child(step.label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .when(step.tone == Tone::Quiet, |text| text.text_color(muted))
                    .child(step.text),
            )
            .children(step.note.map(|note| {
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .pt_0p5()
                    .text_color(if step.tone == Tone::Failed {
                        cx.theme().danger
                    } else {
                        muted
                    })
                    .child(note)
            }));
        if detail.is_some() {
            row = row
                .cursor_pointer()
                .hover(|row| row.bg(cx.theme().accent))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.trajectory_open.remove(&key) {
                        this.trajectory_open.insert(key);
                    }
                    cx.notify();
                }));
        }
        div()
            .v_flex()
            .child(row)
            .children(detail.filter(|_| open).map(|text| {
                div()
                    .ml(px(24.))
                    .mb_1()
                    .p_2()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(text)
            }))
    }
}
