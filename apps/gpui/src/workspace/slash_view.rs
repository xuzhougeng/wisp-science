//! Slash commands in the composer: a message that is only a command is
//! run instead of sent, and a menu lists the commands.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
};

use super::Workspace;
use crate::{
    layers::Layer,
    panel::PanelTab,
    settings::Section,
    slash::{self, Slash, Typed},
};

impl Workspace {
    /// Runs what was typed when it is a command. Returns whether it was
    /// one, in which case nothing is sent.
    pub(super) fn run_typed_slash(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match slash::parse(text) {
            None => false,
            Some(Typed::NotHere(name)) => {
                // What was typed stays, to be changed into a message.
                self.error = Some(crate::i18n::fill(
                    tr("{} is not available in this desktop yet. Type /help for what is."),
                    &[&name],
                ));
                cx.notify();
                true
            }
            Some(Typed::Run(command, rest)) => {
                self.error = None;
                self.state.save_draft(String::new());
                self.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.run_slash(command, rest, window, cx);
                true
            }
        }
    }

    pub(super) fn run_slash(
        &mut self,
        command: Slash,
        rest: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::SlashMenu);
        match command {
            Slash::Review => self.open_review(window, cx),
            Slash::Trajectory => self.open_trajectory(window, cx),
            Slash::Context => self.open_model_view(window, cx),
            Slash::Btw => {
                self.open_aside(window, cx);
                if !rest.is_empty() {
                    self.aside_input
                        .update(cx, |input, cx| input.set_value(rest, window, cx));
                    self.ask_aside(window, cx);
                }
            }
            Slash::Plan => self.toggle_plan_first(window, cx),
            Slash::SaveAsSkill => self.ask_for_skill(window, cx),
            Slash::Skills => {
                self.open_settings(window, cx);
                self.show_settings_section(Section::Skills, window, cx);
            }
            Slash::Files => self.toggle_files(window, cx),
            Slash::Upload => self.attach_files(window, cx),
            Slash::Memo => self.show_panel_tab(PanelTab::Memos, window, cx),
            Slash::Remember => self.show_panel_tab(PanelTab::Memory, window, cx),
            Slash::Archive => self.open_research_archive(window, cx),
            Slash::Compact => self.compact_now(window, cx),
            Slash::Timer => self.set_timer(rest, window, cx),
            Slash::Help => {
                self.layers.open(Layer::SlashMenu);
                self.focus_composer(window, cx);
            }
        }
        cx.notify();
    }

    /// Sets or cancels the open conversation's timer.
    fn set_timer(&mut self, rest: String, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let ask = match slash::timer(&rest) {
            Ok(ask) => ask,
            Err(problem) => {
                // What was typed comes back, to be corrected.
                let typed = format!("/timer {rest}");
                self.state.save_draft(typed.clone());
                self.composer
                    .update(cx, |input, cx| input.set_value(typed, window, cx));
                self.error = Some(problem.to_owned());
                return;
            }
        };
        let said = |this: &mut Self, result: Result<String, String>, cx: &mut Context<Self>| {
            match result {
                Ok(note) => this.notice = Some(note),
                Err(error) => this.error = Some(error),
            }
            cx.notify();
        };
        match ask {
            slash::TimerAsk::Off => {
                let reply = self.backend.cancel_conversation_schedules(project, session);
                cx.spawn_in(window, async move |this, cx| {
                    let result = crate::backend::receive(reply).await.map(|removed| {
                        crate::i18n::fill(tr("Timers cancelled here: {}"), &[&removed])
                    });
                    let _ = this.update(cx, |this, cx| said(this, result, cx));
                })
                .detach();
            }
            slash::TimerAsk::Every { secs, prompt } => {
                let reply = self
                    .backend
                    .create_schedule(wisp_server::desktop::ScheduleInput {
                        project,
                        name: String::new(),
                        prompt,
                        interval_secs: secs,
                        start_at: None,
                        session: Some(session),
                    });
                cx.spawn_in(window, async move |this, cx| {
                    let result = crate::backend::receive(reply).await.map(|()| {
                        crate::i18n::fill(
                            tr("Timer set: {} in this conversation, while the app is open. /timer off cancels it."),
                            &[&crate::schedules::cadence_label(secs)],
                        )
                    });
                    let _ = this.update(cx, |this, cx| said(this, result, cx));
                })
                .detach();
            }
        }
    }

    /// Compacts the open conversation and says what that did.
    fn compact_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        self.notice = Some(tr("Compacting…").to_owned());
        let reply = self.backend.compact_conversation(project, session.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = crate::backend::receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Said only where it was asked.
                if this.state.session.as_ref() == Some(&session) {
                    match result {
                        Ok(done) => {
                            this.notice = Some(slash::compacted_note(
                                done.before,
                                done.after,
                                done.summarised,
                            ))
                        }
                        Err(error) => {
                            this.notice = None;
                            this.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The "/" button of the composer, with the list of commands above it
    /// while that is open.
    pub(super) fn slash_button(&self, cx: &mut Context<Self>) -> Div {
        let open = self.layers.contains(Layer::SlashMenu);
        let button = div().relative().child(
            Button::new("slash-commands")
                .label("/")
                .small()
                .ghost()
                .selected(open)
                .tooltip(tr("Commands: type one as a message, or pick it here"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_composer_menu(Layer::SlashMenu, window, cx)
                })),
        );
        if !open {
            return button;
        }
        let mut menu = Self::composer_menu(380., cx);
        for (name, command, about) in slash::COMMANDS {
            if command == Slash::Help {
                continue;
            }
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("slash{name}")),
                    name,
                    tr(about),
                    false,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.run_slash(command, String::new(), window, cx)
                })),
            );
        }
        button.child(deferred(menu))
    }
}
