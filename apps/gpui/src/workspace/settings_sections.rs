//! The Settings pages besides Models: how turns run (Conversation) and what
//! runs without asking (Permissions). Each change is saved as it is made and
//! applies from the next turn.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt, button::Button, input::Input,
};
use wisp_server::desktop::TurnDefaults;

use super::{
    Workspace,
    shell::{error_banner, glyph, icon_button},
};
use crate::{
    backend::receive,
    layers::Layer,
    settings::{self, Section},
};

/// How wide a setting's name and note are, and the page that holds them.
const NOTE_WIDTH: f32 = 460.;
const PAGE_WIDTH: f32 = 720.;

/// Steps one turn may take; the host's range.
const STEPS: std::ops::RangeInclusive<usize> = 1..=1_000;

/// A setting: its name and what it does on the left, its control on the right.
fn setting(
    title: &'static str,
    note: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    div()
        .h_flex()
        .items_start()
        .justify_between()
        .gap_6()
        .py_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            // A fixed measure: the note wraps to it whatever the control
            // beside it needs.
            div()
                .v_flex()
                .w(px(NOTE_WIDTH))
                .flex_shrink_0()
                .gap_1()
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(note.into()),
                ),
        )
        .child(div().flex_shrink_0().child(control))
}

impl Workspace {
    pub(super) fn show_settings_section(
        &mut self,
        section: Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::ConfirmDeleteModel);
        self.settings_section = section;
        self.settings_error = None;
        match section {
            Section::Models => {}
            Section::Skills => self.load_skills(window, cx),
            Section::Conversation => self.load_turn_defaults(window, cx),
            Section::Permissions => {
                self.load_turn_defaults(window, cx);
                self.load_grants(window, cx);
            }
        }
        cx.notify();
    }

    fn load_turn_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.turn_defaults();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(defaults) => this.accept_turn_defaults(defaults, window, cx),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn accept_turn_defaults(
        &mut self,
        defaults: TurnDefaults,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.turn_defaults = Some(defaults);
        self.steps_input.update(cx, |input, cx| {
            input.set_value(defaults.max_iterations.to_string(), window, cx)
        });
    }

    /// Saves one change to how turns run and shows what the host kept.
    fn save_turn_defaults(
        &mut self,
        defaults: TurnDefaults,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        let reply = self.backend.set_turn_defaults(defaults);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(defaults) => this.accept_turn_defaults(defaults, window, cx),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_steps(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(defaults) = self.turn_defaults else {
            return;
        };
        match settings::steps(&self.steps_input.read(cx).value(), STEPS) {
            Ok(max_iterations) => self.save_turn_defaults(
                TurnDefaults {
                    max_iterations,
                    ..defaults
                },
                window,
                cx,
            ),
            Err(problem) => {
                self.settings_error = Some(problem);
                cx.notify();
            }
        }
    }

    fn load_grants(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.project_grants();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(grants) => this.grants = grants,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn revoke_grant(
        &mut self,
        project: String,
        tool: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        let reply = self.backend.revoke_project_grant(project, tool);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.settings_error = Some(error);
                }
                this.load_grants(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Lists every skill: the open project's and the ones for all projects.
    fn load_skills(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.skills(self.state.project.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(skills) => this.skills = Some(skills),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_skill_enabled(
        &mut self,
        name: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        // Shown at once; the list read afterwards confirms it.
        if let Some(row) = self
            .skills
            .iter_mut()
            .flatten()
            .find(|row| row.name == name)
        {
            row.enabled = enabled;
        }
        let reply = self.backend.set_skill_enabled(name, enabled);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.settings_error = Some(error);
                }
                this.load_skills(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_skill_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(skills) = &self.skills else {
            return page.child(div().text_sm().text_color(muted).child("Loading…"));
        };
        let query = self.skills_search.read(cx).value().to_string();
        let mut page = page
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().child(settings::skills_summary(skills)))
                            .child(div().text_xs().text_color(muted).child(
                                if self.state.project.is_some() {
                                    "The open project's skills and the ones for every project. \
                                     A skill that is off is not offered to the model."
                                } else {
                                    "Skills for every project. Open a project to see its own \
                                     as well. A skill that is off is not offered to the model."
                                },
                            )),
                    )
                    .child(
                        Button::new("skills-reload")
                            .icon(glyph("refresh"))
                            .label("Reload")
                            .small()
                            .flex_shrink_0()
                            .tooltip("Read the skill folders again")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.load_skills(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .py_3()
                    .w(px(320.))
                    .child(Input::new(&self.skills_search).small()),
            );
        let shown: Vec<_> = skills
            .iter()
            .filter(|row| settings::skill_matches(row, &query))
            .collect();
        if shown.is_empty() && !skills.is_empty() {
            page = page.child(div().text_sm().text_color(muted).child("No skill matches."));
        }
        for row in shown {
            let name = row.name.clone();
            let enabled = row.enabled;
            let mut about = settings::skill_source(row.source).to_owned();
            if !row.tags.is_empty() {
                about = format!("{about} · {}", row.tags.join(", "));
            }
            page = page.child(
                div()
                    .h_flex()
                    .items_start()
                    .justify_between()
                    .gap_6()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .v_flex()
                            .w(px(NOTE_WIDTH + 80.))
                            .flex_shrink_0()
                            .gap_1()
                            .when(!enabled, |text| text.opacity(0.6))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(row.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(row.description.clone()),
                            )
                            .child(div().text_xs().text_color(muted).child(about)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("skill-{}", row.name)))
                            .label(if enabled { "On" } else { "Off" })
                            .small()
                            .selected(enabled)
                            .flex_shrink_0()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_skill_enabled(name.clone(), !enabled, window, cx)
                            })),
                    ),
            );
        }
        page
    }

    fn settings_page(&self, cx: &App) -> Div {
        let mut page = div().v_flex().w(px(PAGE_WIDTH)).p_6();
        if let Some(error) = &self.settings_error {
            page = page.child(div().pb_3().child(error_banner(error, cx)));
        }
        page
    }

    pub(super) fn render_conversation_settings(&self, cx: &mut Context<Self>) -> Div {
        let page = self.settings_page(cx);
        let Some(defaults) = self.turn_defaults else {
            return page.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Loading…"),
            );
        };
        let (low, high) = defaults.compact_range;
        let percent = defaults.auto_compact_percent;
        let step = |id: &'static str, name: &'static str, tip: &'static str, up: bool| {
            let next = settings::step_percent(percent, up, defaults.compact_range);
            icon_button(id, name, tip)
                .disabled(!defaults.auto_compact || next == percent)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            auto_compact_percent: next,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                }))
        };
        page.child(setting(
            "Steps per turn",
            format!(
                "How many times the model may call tools or answer within one turn, from {} \
                 to {}. At the limit it stops and says where it got to.",
                STEPS.start(),
                STEPS.end()
            ),
            div()
                .h_flex()
                .gap_2()
                .child(
                    div()
                        .w(px(90.))
                        .child(Input::new(&self.steps_input).small()),
                )
                .child(
                    Button::new("steps-save")
                        .label("Save")
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| this.save_steps(window, cx))),
                ),
            cx,
        ))
        .child(setting(
            "Compact automatically",
            "When the context fills up, older turns are summarised so the conversation \
             can go on. Off, a long conversation ends when the model's window is full.",
            Button::new("auto-compact")
                .label(if defaults.auto_compact { "On" } else { "Off" })
                .small()
                .selected(defaults.auto_compact)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            auto_compact: !defaults.auto_compact,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                })),
            cx,
        ))
        .child(setting(
            "Compact at",
            format!(
                "How full the context is when that happens, from {low}% to {high}%. Lower \
                 compacts sooner and keeps more room for long tool output."
            ),
            div()
                .h_flex()
                .gap_1()
                .child(step("compact-down", "minus", "Compact sooner", false))
                .child(
                    div()
                        .w(px(48.))
                        .text_center()
                        .text_sm()
                        .child(format!("{percent}%")),
                )
                .child(step("compact-up", "plus", "Compact later", true)),
            cx,
        ))
    }

    pub(super) fn render_permission_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(defaults) = self.turn_defaults else {
            return page.child(div().text_sm().text_color(muted).child("Loading…"));
        };
        let mode = |id: &'static str, label: &'static str, full_access: bool| {
            Button::new(id)
                .label(label)
                .small()
                .selected(defaults.full_access == full_access)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            full_access,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                }))
        };
        let mut page = page.child(setting(
            "New conversations start in",
            "Ask first: tools that change files or run commands wait for your answer. \
             Full access: they run at once. A dangerous shell command asks either way. \
             Conversations you already have keep their own choice.",
            div()
                .h_flex()
                .gap_1()
                .child(mode("default-ask", "Ask first", false))
                .child(mode("default-full", "Full access", true)),
            cx,
        ));
        page = page.child(
            div()
                .v_flex()
                .gap_1()
                .pt_4()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Always allowed"),
                )
                .child(div().w(px(NOTE_WIDTH)).text_xs().text_color(muted).child(
                    "Tools you answered \"Always in this project\" for. They run without \
                     asking in every conversation of that project until you take them back.",
                )),
        );
        if self.grants.is_empty() {
            page = page.child(
                div()
                    .pt_3()
                    .text_sm()
                    .text_color(muted)
                    .child("No tool is always allowed."),
            );
        }
        for grants in &self.grants {
            let mut tools = div().h_flex().flex_wrap().gap_2();
            for tool in &grants.tools {
                let (project, name) = (grants.project_id.clone(), tool.clone());
                tools = tools.child(
                    Button::new(SharedString::from(format!(
                        "revoke-{}-{tool}",
                        grants.project_id
                    )))
                    .icon(glyph("close"))
                    .label(tool.clone())
                    .small()
                    .tooltip("Ask again before this tool runs")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.revoke_grant(project.clone(), name.clone(), window, cx)
                    })),
                );
            }
            page = page.child(
                div()
                    .v_flex()
                    .gap_2()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().child(grants.project_name.clone()))
                    .child(tools),
            );
        }
        page
    }
}
