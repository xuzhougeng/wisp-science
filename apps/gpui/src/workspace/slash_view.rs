//! Slash commands in the composer: a message that is only a command is
//! run instead of sent, and a menu lists the commands.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};

use super::{Workspace, shell::glyph, surfaces, tone_color};
use crate::{
    assets::icon,
    layers::Layer,
    panel::PanelTab,
    present::Tone,
    schedules::{TimerAct, TimerCall},
    settings::Section,
    slash::{self, Slash},
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
        let Some((command, rest)) = slash::parse(text) else {
            return false;
        };
        self.error = None;
        self.state.save_draft(String::new());
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.run_slash(command, rest, window, cx);
        true
    }

    pub(super) fn run_slash(
        &mut self,
        command: Slash,
        rest: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::SlashMenu);
        // A conversation that cannot be written to still opens what is
        // there to read; the composer's card says why the rest is not.
        if slash::writes(command) && self.read_only().is_some() {
            return cx.notify();
        }
        match command {
            Slash::Review => self.open_review(window, cx),
            Slash::Trajectory => self.open_trajectory(window, cx),
            Slash::Context => self.open_model_view(window, cx),
            Slash::Btw => {
                self.open_aside(window, cx);
                if !rest.is_empty() {
                    // The aside's field is one line; a line break there
                    // would not draw.
                    let rest = crate::present::one_line(&rest);
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
            Slash::Compact => self.open_compact(cx),
            Slash::Timer => self.set_timer(rest, window, cx),
            // These act on the conversation as it stands: its latest row.
            Slash::Fork => {
                if let Some(last) = self.rows.len().checked_sub(1) {
                    self.branch_from(last, window, cx);
                }
            }
            Slash::Rewind => {
                if let Some(last) = self.rows.len().checked_sub(1) {
                    self.ask_rewind(last, cx);
                }
            }
            Slash::Permission => self.toggle_full_access(window, cx),
            Slash::Share => self.share_conversation_page(window, cx),
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
                self.fill_composer(typed, window, cx);
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
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.load_timer(window, cx);
                        said(this, result, cx)
                    });
                })
                .detach();
            }
            slash::TimerAsk::Every { secs, prompt } => {
                // One timer per conversation: setting it again changes it.
                let reply = self
                    .backend
                    .set_conversation_timer(project, session, prompt, secs);
                cx.spawn_in(window, async move |this, cx| {
                    let result = crate::backend::receive(reply).await.map(|()| {
                        crate::i18n::fill(
                            tr("Timer set: {} in this conversation, while the app is open. /timer off cancels it."),
                            &[&crate::schedules::cadence_label(secs)],
                        )
                    });
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.load_timer(window, cx);
                        said(this, result, cx)
                    });
                })
                .detach();
            }
        }
    }

    /// Reads the open conversation's timer from the host.
    pub(super) fn load_timer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            self.timer = None;
            return;
        };
        if self
            .timer
            .as_ref()
            .is_some_and(|row| row.session.as_deref() != Some(session.as_str()))
        {
            self.timer = None;
        }
        let reply = self.backend.conversation_timer(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = crate::backend::receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                // A timer that cannot be read is left as it was shown.
                if let Ok(timer) = result {
                    this.timer = timer;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Pauses, resumes or cancels the open conversation's timer.
    fn act_on_timer(&mut self, act: TimerAct, window: &mut Window, cx: &mut Context<Self>) {
        let Some(call) = self
            .timer
            .as_ref()
            .and_then(|row| crate::schedules::timer_call(row, act))
        else {
            return;
        };
        if self.timer_busy {
            return;
        }
        self.timer_busy = true;
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = match call {
                TimerCall::SetEnabled { id, enabled } => {
                    crate::backend::receive(backend.set_schedule_enabled(id, enabled)).await
                }
                TimerCall::Cancel { project, session } => {
                    crate::backend::receive(backend.cancel_conversation_schedules(project, session))
                        .await
                        .map(|_| ())
                }
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.timer_busy = false;
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.load_timer(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The strip above the composer while the conversation has a timer:
    /// what it repeats, how often and when next, or that it is paused, with
    /// Pause or Resume and Cancel.
    pub(super) fn timer_strip(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let row = self
            .timer
            .as_ref()
            .filter(|row| row.session.is_some() && row.session == self.state.session)?;
        let muted = cx.theme().muted_foreground;
        let line =
            crate::schedules::timer_line(row, chrono::Utc::now().timestamp(), &chrono::Local);
        let enabled = row.enabled;
        Some(
            surfaces::bar("timer", cx)
                .h_flex()
                .gap_2()
                .child(icon("clock").text_color(if enabled {
                    tone_color(Tone::Active, cx)
                } else {
                    muted
                }))
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .truncate()
                                .child(crate::present::one_line(&row.prompt)),
                        )
                        .child(div().text_xs().text_color(muted).child(line)),
                )
                .child(
                    Button::new("timer-toggle")
                        .icon(glyph(if enabled { "pause" } else { "play" }))
                        .label(if enabled { tr("Pause") } else { tr("Resume") })
                        .xsmall()
                        .ghost()
                        .disabled(self.timer_busy)
                        .tooltip(if enabled {
                            tr("Stop firing; the prompt and the interval are kept")
                        } else {
                            tr("Fire again on the interval; a time missed while paused fires once")
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.act_on_timer(TimerAct::Toggle, window, cx)
                        })),
                )
                .child(
                    Button::new("timer-cancel")
                        .icon(glyph("trash"))
                        .label(tr("Cancel timer"))
                        .xsmall()
                        .ghost()
                        .disabled(self.timer_busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.act_on_timer(TimerAct::Cancel, window, cx)
                        })),
                ),
        )
    }

    /// Opens the `/` menu on a `/` typed where a word starts, narrows it as
    /// the word grows, and shuts it once the word is gone.
    pub(super) fn follow_slash(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(word) = slash::typed(text).map(str::to_owned) else {
            self.slash_quiet = false;
            if self.slash_typed.take().is_some() {
                self.layers.close(Layer::SlashMenu);
            }
            return;
        };
        if self.slash_quiet || self.state.session.is_none() {
            return;
        }
        if !self.layers.contains(Layer::SlashMenu) {
            for other in Layer::COMPOSER_MENUS {
                self.layers.close(other);
            }
            self.layers.open(Layer::SlashMenu);
        }
        if self.slash_typed.as_deref() != Some(word.as_str()) {
            self.slash_typed = Some(word.clone());
            self.load_slash_skills(word, window, cx);
        }
    }

    /// The skills the `/` menu offers for `word`.
    fn load_slash_skills(&mut self, word: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let reply = self.backend.skill_options(project, word.clone(), 8);
        cx.spawn_in(window, async move |this, cx| {
            let found = crate::backend::receive(reply).await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                // Only for the word the menu still follows.
                if this.slash_typed.as_deref().unwrap_or_default() == word {
                    this.slash_skills = found;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// What the composer keeps when something from the `/` menu takes the
    /// place of the word typed after `/`.
    fn slash_rest(&mut self, cx: &mut Context<Self>) -> String {
        let text = self.composer.read(cx).value().to_string();
        match self.slash_typed.take() {
            Some(_) => slash::without_typed(&text),
            None => text,
        }
    }

    /// Runs a command picked from the `/` menu, or writes it out when it
    /// needs words after it.
    fn pick_slash(&mut self, command: Slash, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.slash_typed.is_some();
        let rest = self.slash_rest(cx);
        self.layers.close(Layer::SlashMenu);
        if slash::takes_words(command) {
            let name = slash::COMMANDS
                .iter()
                .find(|(_, which, _)| *which == command)
                .map_or("", |(name, ..)| *name);
            let written = format!("{name} {}", rest.trim_start());
            self.state.save_draft(written.clone());
            self.fill_composer(written, window, cx);
            return cx.notify();
        }
        if typed {
            self.state.save_draft(rest.clone());
            self.fill_composer(rest, window, cx);
        }
        self.run_slash(command, String::new(), window, cx);
    }

    /// Attaches a skill picked from the `/` menu to the message.
    fn pick_slash_skill(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.slash_typed.is_some();
        let rest = self.slash_rest(cx);
        self.layers.close(Layer::SlashMenu);
        if typed {
            self.state.save_draft(rest.clone());
            self.fill_composer(rest, window, cx);
        }
        self.attach(crate::compose::Chip::Skill(name), cx);
        self.focus_composer(window, cx);
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
                .tooltip(tr("Commands and skills: type / or pick one here"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.slash_typed = None;
                    this.toggle_composer_menu(Layer::SlashMenu, window, cx);
                    if this.layers.contains(Layer::SlashMenu) {
                        this.load_slash_skills(String::new(), window, cx);
                    }
                })),
        );
        if !open {
            return button;
        }
        let mut menu = Self::composer_menu(380., cx);
        // An ACP agent's conversation lists the agent's own commands; one
        // is written into the message, to be sent with what follows it.
        if !self.options().acp_agent.is_empty() {
            let commands = self
                .status
                .as_ref()
                .and_then(|status| status.acp.as_ref())
                .map(|controls| controls.commands.clone())
                .unwrap_or_default();
            if commands.is_empty() {
                menu = menu.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("The agent lists its commands once it has answered.")),
                );
            }
            let word = self.slash_typed.clone().unwrap_or_default();
            for command in commands
                .into_iter()
                .filter(|command| command.name.starts_with(&word))
            {
                let detail = match command.hint.trim() {
                    "" => command.description.clone(),
                    hint => format!("{} — {hint}", command.description),
                };
                let typed = format!("/{} ", command.name);
                menu = menu.child(
                    Self::menu_row(
                        SharedString::from(format!("agent-slash-{}", command.name)),
                        format!("/{}", command.name),
                        detail,
                        false,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // Typed after other words, the command joins them.
                        let rest = match this.slash_typed.is_some() {
                            true => this.slash_rest(cx),
                            false => String::new(),
                        };
                        this.layers.close(Layer::SlashMenu);
                        this.fill_composer(format!("{rest}{typed}"), window, cx);
                    })),
                );
            }
            return button.child(deferred(menu));
        }
        let word = self.slash_typed.clone().unwrap_or_default();
        let muted = cx.theme().muted_foreground;
        let mut rows = div()
            .id("slash-rows")
            .v_flex()
            .max_h(px(380.))
            .overflow_y_scroll();
        let commands = slash::matching(&word);
        for (name, command, about) in &commands {
            let command = *command;
            rows = rows.child(
                Self::menu_row(
                    SharedString::from(format!("slash{name}")),
                    *name,
                    tr(about),
                    false,
                    cx,
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.pick_slash(command, window, cx)),
                ),
            );
        }
        // The WebView's `/` lists skills: picking one attaches it.
        let skills: Vec<_> = self
            .slash_skills
            .iter()
            .filter_map(|option| match &option.reference {
                wisp_dto::ComposerReferenceArg::Skill { name } => {
                    Some((name.clone(), option.detail.clone()))
                }
                _ => None,
            })
            .collect();
        if !skills.is_empty() {
            rows = rows.child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Skills, to attach")),
            );
        }
        for (name, detail) in &skills {
            let picked = name.clone();
            rows = rows.child(
                Self::menu_row(
                    SharedString::from(format!("slash-skill-{name}")),
                    format!("/{name}"),
                    detail.clone(),
                    false,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.pick_slash_skill(picked.clone(), window, cx)
                })),
            );
        }
        if commands.is_empty() && skills.is_empty() {
            rows = rows.child(div().px_2().py_1().text_xs().text_color(muted).child(
                crate::i18n::fill(tr("No command or skill begins with /{}"), &[&word]),
            ));
        }
        button.child(deferred(menu.child(rows)))
    }
}
