//! The research assistant in the window: opening its one conversation,
//! and the bar over it with today's plan.
use crate::i18n::tr;
use chrono::Local;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, StyledExt};

use super::{
    Workspace, chip,
    shell::{glyph, icon_button},
    tone_color,
};
use crate::{assistant, backend::receive, layers::Layer, present::Tone};

impl Workspace {
    /// Opens the assistant's conversation, making it on first use.
    pub(super) fn open_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::Palette);
        self.error = None;
        let reply = self.backend.open_assistant();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((project, session)) => {
                        this.open_session(project, session, window, cx);
                        this.load_assistant_plan(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads today's plan, with earlier items still open.
    pub(super) fn load_assistant_plan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let reply = self.backend.assistant_plan(today);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // A plan that cannot be read leaves the bar without one.
                this.assistant_plan = result.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    fn set_plan_item(
        &mut self,
        id: String,
        status: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply = self.backend.set_assistant_plan_item(id, status.into());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.load_assistant_plan(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Over the assistant's conversation: what it is for, and today's plan
    /// with a way to tick items off or drop them.
    pub(super) fn render_assistant_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let session = self.state.session.as_deref()?;
        if !wisp_server::desktop::is_assistant(session) {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let mut bar = div()
            .v_flex()
            .gap_1()
            .px_4()
            .py_2()
            .flex_shrink_0()
            .bg(tone_color(Tone::Neutral, cx).opacity(0.06))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(glyph("bot").text_color(muted))
                    .child(div().flex_1().text_xs().text_color(muted).child(tr(
                        "Your research assistant keeps track of every project, remembers your \
                         plan and hands work to project conversations; it does no work itself.",
                    ))),
            )
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .child(assistant::summary(&self.assistant_plan)),
            );
        for (index, item) in self.assistant_plan.iter().enumerate() {
            let (status, tone) = assistant::status(&item.status);
            let mut row = div().h_flex().gap_2().child(chip(status, tone, cx)).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .child(item.title.clone()),
            );
            if let Some(project) = &item.project_name {
                row = row.child(chip(project.clone(), Tone::Neutral, cx));
            }
            if item.status == "open" {
                let done = item.id.clone();
                let dropped = item.id.clone();
                row = row
                    .child(
                        icon_button(("plan-done", index), "check", "Mark it done")
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_plan_item(done.clone(), "done", window, cx)
                            })),
                    )
                    .child(
                        icon_button(("plan-drop", index), "ban", "Drop it")
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_plan_item(dropped.clone(), "dropped", window, cx)
                            })),
                    );
            }
            bar = bar.child(row);
        }
        Some(bar)
    }
}
