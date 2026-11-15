//! The Settings pages besides Models: how turns run (Conversation) and what
//! runs without asking (Permissions). Each change is saved as it is made and
//! applies from the next turn.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
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

/// Names the General page's choices are saved under.
const APPEARANCE: &str = "appearance";
const LANGUAGE: &str = "language";
const LAUNCH: &str = "launch";
const SEND_KEY: &str = "send_key";
const SIDEBAR: &str = "sidebar";
const FILES_SORT: &str = "files_sort";
/// The Files tab as tiles (`grid`) or rows.
const FILES_VIEW: &str = "files_view";
/// The side panel's tabs that have not been taken away.
const PANEL_TABS: &str = "panel_tabs";
const NOTIFY: &str = "notifications";
const NOTIFY_SOUND: &str = "notification_sound";
const LAST_OPEN: &str = "last_open";

/// Set once the check at start has been made, so a second window does not
/// make it again.
static CHECKED_AT_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How wide a setting's name and note are, and the page that holds them.
const NOTE_WIDTH: f32 = 460.;
const PAGE_WIDTH: f32 = 720.;

/// Steps one turn may take; the host's range.
const STEPS: std::ops::RangeInclusive<usize> = 1..=1_000;

/// A setting: its name and what it does on the left, its control on the right.
pub(super) fn setting(
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
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(tr(title)),
                )
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
            Section::Models => self.load_subscriptions(window, cx),
            Section::General => {}
            Section::Skills => {
                self.load_skills(window, cx);
                if self.community_skills.is_none() {
                    self.load_community_skills(false, window, cx);
                }
            }
            Section::Connections => {
                self.load_connectors(window, cx);
                self.load_mcp_servers(window, cx);
                self.load_approvals(window, cx);
            }
            Section::Credentials => self.load_credentials(window, cx),
            Section::Environments => self.load_environments(window, cx),
            Section::Experts => {
                self.load_experts(window, cx);
                // What an expert can be limited to.
                self.load_skills(window, cx);
                self.load_connectors(window, cx);
                self.load_mcp_servers(window, cx);
            }
            Section::AcpAgents => self.load_acp_agents(window, cx),
            Section::QuickActions => {
                self.load_quick_actions(window, cx);
                // An action names the workflow it starts.
                self.load_workflows(window, cx);
            }
            Section::Pet => self.load_pet(window, cx),
            Section::Browser => self.load_browser(window, cx),
            Section::Workflows => self.load_workflows(window, cx),
            Section::Remote => {
                self.load_remote_access(window, cx);
                self.load_sync_settings(window, cx);
                self.load_device_bridge(window, cx);
            }
            Section::Channels => self.load_channels(window, cx),
            Section::Plugins => self.load_plugins(window, cx),
            Section::DecisionModel => self.load_decision_model(window, cx),
            Section::Network => self.load_network(window, cx),
            Section::Usage => self.load_usage(window, cx),
            Section::Storage => self.load_storage(window, cx),
            Section::Conversation => self.load_turn_defaults(window, cx),
            Section::Hooks => self.load_hooks(window, cx),
            Section::Permissions => {
                self.load_turn_defaults(window, cx);
                self.load_grants(window, cx);
                self.load_approvals(window, cx);
            }
        }
        cx.notify();
    }

    /// Chooses the model sub-agents work on and shows what was kept.
    fn save_subagent_model(
        &mut self,
        profile: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        let reply = self.backend.set_subagent_model(profile);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(kept) => this.subagent_model = kept,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Chooses the model that describes pictures and shows what was kept.
    fn save_picture_model(&mut self, profile: String, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_error = None;
        let reply = self.backend.set_picture_model(profile);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(kept) => this.picture_model = kept,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Chooses the model reviews are made with and shows what was kept.
    fn save_review_model(&mut self, profile: String, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_error = None;
        let reply = self.backend.set_review_model(profile);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(kept) => this.review_model = kept,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_turn_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let failures = self.backend.failure_analysis();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(failures) = receive(failures).await {
                let _ = this.update(cx, |this, cx| {
                    this.failure_analysis = Some(failures);
                    cx.notify();
                });
            }
        })
        .detach();
        let chosen = self.backend.review_model();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(chosen) = receive(chosen).await {
                let _ = this.update(cx, |this, cx| {
                    this.review_model = chosen;
                    cx.notify();
                });
            }
        })
        .detach();
        let worker = self.backend.subagent_model();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(worker) = receive(worker).await {
                let _ = this.update(cx, |this, cx| {
                    this.subagent_model = worker;
                    cx.notify();
                });
            }
        })
        .detach();
        let reader = self.backend.picture_model();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(reader) = receive(reader).await {
                let _ = this.update(cx, |this, cx| {
                    this.picture_model = reader;
                    cx.notify();
                });
            }
        })
        .detach();
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

    /// Reads the open project's approval mode and tool rules; none when no
    /// project is open.
    fn load_approvals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            self.approvals = None;
            return;
        };
        let reply = self.backend.project_approvals(project.clone());
        self.keep_approvals(project, reply, window, cx);
    }

    /// Shows what the host kept for `project`'s approvals.
    fn keep_approvals(
        &mut self,
        project: String,
        reply: crate::backend::Reply<wisp_server::desktop::ProjectApprovals>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(kept) => this.approvals = Some((project, kept)),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_approval_scope(
        &mut self,
        scope: wisp_server::desktop::ApprovalScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.approvals.as_ref().map(|(project, _)| project.clone()) else {
            return;
        };
        let reply = self.backend.set_approval_scope(project.clone(), scope);
        self.keep_approvals(project, reply, window, cx);
    }

    fn save_tool_rule(
        &mut self,
        tool: String,
        rule: Option<wisp_server::desktop::ToolRule>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.approvals.as_ref().map(|(project, _)| project.clone()) else {
            return;
        };
        let reply = self.backend.set_tool_rule(project.clone(), tool, rule);
        self.keep_approvals(project, reply, window, cx);
    }

    /// The open project's approval mode, and the tools that have a rule of
    /// their own, on the Permissions page.
    fn approval_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut section = div().v_flex().gap_2().pt_4().child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(tr("Tool approval mode")),
        );
        let Some((project, approvals)) = &self.approvals else {
            return section.child(
                div()
                    .w(px(NOTE_WIDTH))
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Open a project to choose how its tools are approved.")),
            );
        };
        let name = self
            .projects
            .iter()
            .find(|row| &row.id == project)
            .map_or_else(|| project.clone(), |row| row.name.clone());
        let mut modes = div().h_flex().gap_1();
        for (scope, label, _) in settings::APPROVAL_SCOPES {
            modes = modes.child(
                Button::new(SharedString::from(format!("approval-scope-{label}")))
                    .label(tr(label))
                    .small()
                    .selected(approvals.scope == scope)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_approval_scope(scope, window, cx)
                    })),
            );
        }
        section = section
            .child(div().text_xs().text_color(muted).child(crate::i18n::fill(
                tr("For {}, from the next turn."),
                &[&name],
            )))
            .child(modes)
            .child(
                div()
                    .w(px(NOTE_WIDTH))
                    .text_xs()
                    .text_color(muted)
                    .child(settings::approval_note(approvals.scope)),
            );
        if !approvals.rules.is_empty() {
            let mut rules = div().h_flex().flex_wrap().gap_2();
            for (tool, rule) in &approvals.rules {
                let name = tool.clone();
                let said = match rule {
                    wisp_server::desktop::ToolRule::Ask => tr("Ask first"),
                    wisp_server::desktop::ToolRule::Deny => tr("Deny"),
                };
                rules = rules.child(
                    Button::new(SharedString::from(format!("tool-rule-{tool}")))
                        .icon(glyph("close"))
                        .label(format!("{tool} · {said}"))
                        .small()
                        .tooltip(tr("Allow it again"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.save_tool_rule(name.clone(), None, window, cx)
                        })),
                );
            }
            section = section
                .child(div().text_xs().text_color(muted).child(tr(
                    "Tools with a rule of their own, set on the Connections page:",
                )))
                .child(rules);
        }
        section
    }

    /// The open project's switch that lets every tool of a connector run
    /// whatever its rule; none without an open project.
    fn connector_skip(&self, connector: &str, cx: &mut Context<Self>) -> Option<Div> {
        let (_, approvals) = self.approvals.as_ref()?;
        let on = approvals.skip.contains(connector);
        let domain = connector.to_owned();
        Some(
            div().h_flex().child(
                Button::new(SharedString::from(format!("connector-skip-{connector}")))
                    .label(tr("Skip approvals"))
                    .xsmall()
                    .selected(on)
                    .tooltip(tr("Its tools run without asking, whatever their rules"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_connector_skip(domain.clone(), !on, window, cx)
                    })),
            ),
        )
    }

    fn save_connector_skip(
        &mut self,
        connector: String,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.approvals.as_ref().map(|(project, _)| project.clone()) else {
            return;
        };
        let reply = self
            .backend
            .set_connector_skip(project.clone(), connector, on);
        self.keep_approvals(project, reply, window, cx);
    }

    /// A connector's tools with their rules for the open project, or their
    /// names alone when no project is open.
    fn connector_tool_rules(
        &self,
        connector: &str,
        tools: &[String],
        cx: &mut Context<Self>,
    ) -> Div {
        let mono = cx.theme().mono_font_family.clone();
        let Some((_, approvals)) = &self.approvals else {
            return div().text_xs().font_family(mono).child(tools.join("  "));
        };
        // Skipped, the rules do not apply.
        let skipped = approvals.skip.contains(connector);
        let mut list = div().v_flex().gap_1();
        for tool in tools {
            let current = approvals.rules.get(tool).copied();
            let mut row = div().h_flex().gap_2().child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .font_family(mono.clone())
                    .child(tool.clone()),
            );
            for (rule, label) in settings::TOOL_RULES {
                let name = tool.clone();
                row = row.child(
                    Button::new(SharedString::from(format!("rule-{tool}-{label}")))
                        .label(tr(label))
                        .xsmall()
                        .ghost()
                        .selected(current == rule)
                        .disabled(skipped)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.save_tool_rule(name.clone(), rule, window, cx)
                        })),
                );
            }
            list = list.child(row);
        }
        list
    }

    fn load_grants(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let everywhere = self.backend.global_grants();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(everywhere).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(tools) => this.global_grants = tools,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
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

    fn revoke_global_grant(&mut self, tool: String, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_error = None;
        let reply = self.backend.revoke_global_grant(tool);
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
    pub(super) fn load_skills(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Opens the field for a skill's tags, with the ones it has now.
    fn edit_skill_tags(
        &mut self,
        name: String,
        tags: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = tags.join(", ");
        self.skill_tags_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        self.skill_tags_editing = Some(name);
        self.layers.open(Layer::SkillTags);
        cx.notify();
    }

    /// Keeps the tags typed for the skill being changed.
    fn save_skill_tags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.skill_tags_editing.clone() else {
            return;
        };
        let tags = settings::parse_tags(&self.skill_tags_input.read(cx).value());
        self.settings_error = None;
        let reply = self.backend.set_skill_tags(name, tags);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.skill_tags_editing = None;
                        this.layers.close(Layer::SkillTags);
                        this.load_skills(window, cx);
                    }
                    // The field stays open with what was typed.
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_skill_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(skills) = &self.skills else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
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
                                    tr("The open project's skills and the ones for every project. \
                                     A skill that is off is not offered to the model.")
                                } else {
                                    tr("Skills for every project. Open a project to see its own \
                                     as well. A skill that is off is not offered to the model.")
                                },
                            )),
                    )
                    .child(
                        Button::new("skills-reload")
                            .icon(glyph("refresh"))
                            .label(tr("Reload"))
                            .small()
                            .flex_shrink_0()
                            .tooltip(tr("Read the skill folders again"))
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
            page = page.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("No skill matches.")),
            );
        }
        for row in shown {
            let name = row.name.clone();
            let enabled = row.enabled;
            let editing = self.layers.contains(Layer::SkillTags)
                && self.skill_tags_editing.as_deref() == Some(row.name.as_str());
            let (tagged, current) = (row.name.clone(), row.tags.clone());
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
                            .child(div().text_xs().text_color(muted).child(about))
                            .when(editing, |about| {
                                about.child(
                                    div()
                                        .h_flex()
                                        .gap_1()
                                        .pt_1()
                                        .child(
                                            div()
                                                .w(px(260.))
                                                .child(Input::new(&self.skill_tags_input).small()),
                                        )
                                        .child(
                                            Button::new("skill-tags-save")
                                                .label(tr("Save"))
                                                .small()
                                                .primary()
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.save_skill_tags(window, cx)
                                                })),
                                        )
                                        .child(
                                            Button::new("skill-tags-cancel")
                                                .label(tr("Cancel"))
                                                .small()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.skill_tags_editing = None;
                                                    this.layers.close(Layer::SkillTags);
                                                    cx.notify();
                                                })),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_1()
                            .flex_shrink_0()
                            .child(
                                Button::new(SharedString::from(format!("skill-tags-{}", row.name)))
                                    .label(tr("Tags"))
                                    .small()
                                    .ghost()
                                    .tooltip(tr(
                                        "Give it your own tags in place of the ones it came with",
                                    ))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.edit_skill_tags(tagged.clone(), &current, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("skill-{}", row.name)))
                                    .label(if enabled { tr("On") } else { tr("Off") })
                                    .small()
                                    .selected(enabled)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.set_skill_enabled(name.clone(), !enabled, window, cx)
                                    })),
                            ),
                    ),
            );
        }
        page.child(self.skill_store_section(cx))
    }

    fn load_connectors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.connectors();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(connectors) => this.connectors = Some(connectors),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_connector_enabled(
        &mut self,
        domain: String,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        // Shown at once; the list read afterwards confirms it.
        if let Some(row) = self
            .connectors
            .iter_mut()
            .flatten()
            .find(|row| row.domain == domain)
        {
            row.enabled = enabled;
        }
        let reply = self.backend.set_connector_enabled(domain, enabled);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.settings_error = Some(error);
                }
                this.load_connectors(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_connection_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(connectors) = &self.connectors else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let query = self.connectors_search.read(cx).value().to_string();
        let mut page = page
            .child(self.render_mcp_servers(cx))
            .child(
                div()
                    .v_flex()
                    .w(px(NOTE_WIDTH + 80.))
                    .child(
                        div()
                            .text_sm()
                            .child(settings::connectors_summary(connectors)),
                    )
                    .child(div().text_xs().text_color(muted).child(tr(
                        "Literature and biology sources that ship with Wisp. The model can \
                         only read through them. A connector that is off gives it no tools.",
                    ))),
            )
            .child(
                div()
                    .py_3()
                    .w(px(320.))
                    .child(Input::new(&self.connectors_search).small()),
            );
        let shown: Vec<_> = connectors
            .iter()
            .filter(|row| settings::connector_matches(row, &query))
            .collect();
        if shown.is_empty() {
            page = page.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("No connector matches.")),
            );
        }
        for row in shown {
            let enabled = row.enabled;
            let open = self.open_connector.as_deref() == Some(row.domain.as_str());
            let (toggling, unfolding) = (row.domain.clone(), row.domain.clone());
            let mut about = div()
                .v_flex()
                .w(px(NOTE_WIDTH + 80.))
                .flex_shrink_0()
                .gap_1()
                .when(!enabled, |text| text.opacity(0.6))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(row.domain.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(row.description.clone()),
                )
                // In a row of its own, so it keeps its width and its place
                // at the left instead of stretching across the column.
                .child(
                    div().h_flex().child(
                        Button::new(SharedString::from(format!(
                            "connector-tools-{}",
                            row.domain
                        )))
                        .icon(glyph(if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        }))
                        .label(settings::tool_count(row.tools.len()))
                        .xsmall()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_connector = (this.open_connector.as_deref()
                                != Some(unfolding.as_str()))
                            .then(|| unfolding.clone());
                            cx.notify();
                        })),
                    ),
                );
            if open {
                about = about
                    .children(self.connector_skip(&row.domain, cx))
                    .child(self.connector_tool_rules(&row.domain, &row.tools, cx));
                for (index, (label, url)) in row.links.iter().enumerate() {
                    let target = url.clone();
                    about = about.child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format!("{label}: {url}")),
                            )
                            .child(
                                icon_button(
                                    SharedString::from(format!(
                                        "connector-link-{}-{index}",
                                        row.domain
                                    )),
                                    "external-link",
                                    tr("Open in your browser"),
                                )
                                .xsmall()
                                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&target))),
                            ),
                    );
                }
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
                    .child(about)
                    .child(
                        Button::new(SharedString::from(format!("connector-{}", row.domain)))
                            .label(if enabled { tr("On") } else { tr("Off") })
                            .small()
                            .selected(enabled)
                            .flex_shrink_0()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_connector_enabled(toggling.clone(), !enabled, window, cx)
                            })),
                    ),
            );
        }
        page
    }

    /// Reads the choices of the General page and acts on them: wears the
    /// chosen palette and, if asked to, reopens the conversation that was
    /// open last time.
    pub(super) fn load_general(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (banner, sound) = (
            self.backend.preference(NOTIFY),
            self.backend.preference(NOTIFY_SOUND),
        );
        cx.spawn_in(window, async move |this, cx| {
            let saved = |reply| async move { receive(reply).await.ok().flatten() };
            let (banner, sound) = (saved(banner).await, saved(sound).await);
            let _ = this.update(cx, |this, cx| {
                this.notify =
                    crate::notify::Choice::from_saved(banner.as_deref(), sound.as_deref());
                cx.notify();
            });
        })
        .detach();
        // A newer version is looked for once, when the app starts, unless
        // that was turned off; only one that is newer is shown.
        let at_start = self.backend.update_check_at_start();
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let enabled = receive(at_start).await.unwrap_or(true);
            let _ = this.update(cx, |this, cx| {
                this.update_at_start = enabled;
                cx.notify();
            });
            if !enabled || CHECKED_AT_START.swap(true, std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            if let Ok(found) = receive(backend.check_for_update()).await {
                let _ = this.update(cx, |this, cx| {
                    if found.newer {
                        this.update_banner = Some(found);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        // The composer offers them from the start.
        self.load_experts(window, cx);
        self.load_acp_agents(window, cx);
        self.load_quick_actions(window, cx);
        self.load_pet(window, cx);
        let language = self.backend.preference(LANGUAGE);
        cx.spawn_in(window, async move |this, cx| {
            let saved = receive(language).await.ok().flatten();
            let _ = this.update_in(cx, |this, window, cx| {
                this.language =
                    crate::i18n::Language::from_key(saved.as_deref().unwrap_or_default());
                crate::i18n::apply(this.language);
                this.word_fields(window, cx);
                // Everything on screen is worded again.
                cx.refresh_windows();
            });
        })
        .detach();
        let (appearance, launch, place, send_key, sidebar, files_sort) = (
            self.backend.preference(APPEARANCE),
            self.backend.preference(LAUNCH),
            self.backend.preference(LAST_OPEN),
            self.backend.preference(SEND_KEY),
            self.backend.preference(SIDEBAR),
            self.backend.preference(FILES_SORT),
        );
        let files_view = self.backend.preference(FILES_VIEW);
        let panel_tabs = self.backend.preference(PANEL_TABS);
        cx.spawn_in(window, async move |this, cx| {
            let saved = |reply| async move { receive(reply).await.ok().flatten() };
            let (appearance, launch, place, send_key, sidebar, files_sort) = (
                saved(appearance).await,
                saved(launch).await,
                saved(place).await,
                saved(send_key).await,
                saved(sidebar).await,
                saved(files_sort).await,
            );
            let files_view = saved(files_view).await;
            let panel_tabs = saved(panel_tabs).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.send_key =
                    settings::SendKey::from_key(send_key.as_deref().unwrap_or_default());
                this.sidebar_hidden = sidebar.as_deref() == Some("hidden");
                this.files.set_sort(crate::files::FileSort::from_key(
                    files_sort.as_deref().unwrap_or_default(),
                ));
                this.files.grid = files_view.as_deref() == Some("grid");
                this.kept_tabs = crate::panel::decode_tabs(panel_tabs.as_deref());
                this.word_fields(window, cx);
                this.appearance =
                    crate::theme::Appearance::from_key(appearance.as_deref().unwrap_or_default());
                this.launch = settings::Launch::from_key(launch.as_deref().unwrap_or_default());
                crate::theme::apply(this.appearance, window, cx);
                let place = place.as_deref().and_then(settings::decode_place);
                if let (settings::Launch::Resume, Some((project, session))) = (this.launch, place) {
                    // Only on a fresh start: the user may already be somewhere.
                    if this.state.project.is_none() {
                        this.open_session(project, session, window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Looks for a newer version when the app starts, or stops doing so.
    fn set_update_at_start(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.update_at_start = enabled;
        if !enabled {
            self.update_banner = None;
        }
        let reply = self.backend.set_update_check_at_start(enabled);
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = receive(reply).await {
                let _ = this.update(cx, |this, cx| {
                    this.settings_error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// The newer version found at start: where to get it, and how to stop
    /// being told.
    pub(super) fn render_update_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let found = self.update_banner.as_ref()?;
        let url = found.url.clone();
        Some(
            div()
                .absolute()
                .bottom_4()
                .left_4()
                .w(px(320.))
                .v_flex()
                .gap_2()
                .p_3()
                .rounded(cx.theme().radius_lg)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .shadow_lg()
                .text_sm()
                .child(crate::i18n::fill(
                    tr("Wisp {} is available; this is {}."),
                    &[&found.latest, &found.current],
                ))
                .child(
                    div()
                        .h_flex()
                        .gap_1()
                        .child(
                            Button::new("update-banner-open")
                                .label(tr("Open releases"))
                                .small()
                                .primary()
                                .tooltip(tr("Open the release page in your browser"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.open_url(&url);
                                    this.update_banner = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("update-banner-never")
                                .label(tr("Don't check at start"))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_update_at_start(false, window, cx)
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            super::shell::icon_button(
                                "update-banner-close",
                                "close",
                                tr("Not now"),
                            )
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.update_banner = None;
                                cx.notify();
                            })),
                        ),
                ),
        )
    }

    /// Asks the release page whether a newer version has been published.
    fn check_for_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update = Some(settings::Update::Checking);
        let reply = self.backend.check_for_update();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.update = Some(match result {
                    Ok(found) => settings::Update::Found(found),
                    Err(problem) => settings::Update::Failed(problem),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Saves one choice of the General page; a failure is shown there.
    /// Remembers how the Files tab lists a folder, for the next start.
    /// Remembers whether the Files tab shows tiles, for the next start.
    pub(super) fn save_files_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = if self.files.grid { "grid" } else { "list" };
        self.save_general(FILES_VIEW, value.into(), window, cx);
    }

    /// Remembers which of the side panel's tabs are kept, for the next start.
    pub(super) fn save_panel_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = crate::panel::encode_tabs(&self.kept_tabs);
        self.save_general(PANEL_TABS, value, window, cx);
    }

    pub(super) fn save_files_sort(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.files.sort.key().to_owned();
        self.save_general(FILES_SORT, value, window, cx);
    }

    /// Puts the project's sidebar away or brings it back, and remembers
    /// which for the next start.
    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_hidden = !self.sidebar_hidden;
        let value = if self.sidebar_hidden {
            "hidden"
        } else {
            "shown"
        };
        self.save_general(SIDEBAR, value.into(), window, cx);
        cx.notify();
    }

    fn save_general(
        &mut self,
        name: &'static str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply = self.backend.set_preference(name, value);
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = receive(reply).await {
                let _ = this.update(cx, |this, cx| {
                    this.settings_error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Notes which conversation is open, for the next start.
    pub(super) fn remember_place(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) = (&self.state.project, &self.state.session) else {
            return;
        };
        let place = settings::encode_place(project, session);
        let reply = self.backend.set_preference(LAST_OPEN, place);
        // Nothing to say if this fails: the app simply opens on Home.
        cx.spawn_in(window, async move |_, _| drop(receive(reply).await))
            .detach();
    }

    pub(super) fn render_general_settings(&self, cx: &mut Context<Self>) -> Div {
        let mut looks = div().h_flex().gap_1();
        for choice in crate::theme::Appearance::ALL {
            looks = looks.child(
                Button::new(SharedString::from(format!("appearance-{}", choice.key())))
                    .label(tr(choice.label()))
                    .small()
                    .selected(self.appearance == choice)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.appearance = choice;
                        crate::theme::apply(choice, window, cx);
                        this.save_general(APPEARANCE, choice.key().into(), window, cx);
                    })),
            );
        }
        let mut languages = div().h_flex().gap_1();
        for choice in crate::i18n::Language::ALL {
            languages = languages.child(
                Button::new(SharedString::from(format!("language-{}", choice.key())))
                    // "System" is a word of the interface; the languages
                    // are named in themselves.
                    .label(tr(choice.label()))
                    .small()
                    .selected(self.language == choice)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.language = choice;
                        crate::i18n::apply(choice);
                        this.word_fields(window, cx);
                        this.save_general(LANGUAGE, choice.key().into(), window, cx);
                        cx.refresh_windows();
                    })),
            );
        }
        let mut sends = div().h_flex().gap_1();
        for choice in settings::SendKey::ALL {
            sends = sends.child(
                Button::new(SharedString::from(format!("send-{}", choice.key())))
                    .label(choice.label())
                    .small()
                    .selected(self.send_key == choice)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.send_key = choice;
                        // The empty composer says which key it is now.
                        this.word_fields(window, cx);
                        this.save_general(SEND_KEY, choice.key().into(), window, cx);
                    })),
            );
        }
        let mut starts = div().h_flex().gap_1();
        for choice in settings::Launch::ALL {
            starts = starts.child(
                Button::new(SharedString::from(format!("launch-{}", choice.key())))
                    .label(choice.label())
                    .small()
                    .selected(self.launch == choice)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.launch = choice;
                        this.save_general(LAUNCH, choice.key().into(), window, cx);
                    })),
            );
        }
        self.settings_page(cx)
            .child(setting(
                tr("Language"),
                tr(
                    "The language of the interface. System follows your computer's language: \
                 Chinese when it is set to Chinese, English otherwise. What the model \
                 writes is not affected.",
                ),
                languages,
                cx,
            ))
            .child(setting(
                tr("Appearance"),
                tr("System follows your computer's light or dark setting and changes with it."),
                looks,
                cx,
            ))
            .child(setting(tr("Send with"), settings::send_key_note(), sends, cx))
            .child(setting(
                tr("Notify me"),
                tr(
                    "When a turn ends or a tool waits for approval while you are in another app, post a system notification. Windows has none without an installed app, so only the sound is available there.",
                ),
                Button::new("notify-banner")
                    .label(if self.notify.banner { tr("On") } else { tr("Off") })
                    .small()
                    .selected(self.notify.banner)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.notify.banner = !this.notify.banner;
                        let value = this.notify.banner.to_string();
                        this.save_general(NOTIFY, value, window, cx);
                    })),
                cx,
            ))
            .child(setting(
                tr("Play a sound"),
                tr("Also play a sound then, whether or not a notification is posted."),
                Button::new("notify-sound")
                    .label(if self.notify.sound { tr("On") } else { tr("Off") })
                    .small()
                    .selected(self.notify.sound)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.notify.sound = !this.notify.sound;
                        let value = this.notify.sound.to_string();
                        this.save_general(NOTIFY_SOUND, value, window, cx);
                    })),
                cx,
            ))
            .child({
                let (note, available) =
                    settings::update_note(self.update.as_ref(), self.update_at_start);
                let mut actions = div().h_flex().gap_1().child(
                    Button::new("update-check")
                        .label(tr("Check for updates"))
                        .small()
                        .disabled(self.update == Some(settings::Update::Checking))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.check_for_update(window, cx)),
                        ),
                );
                let at_start = self.update_at_start;
                actions = actions.child(
                    Button::new("update-at-start")
                        .label(if at_start {
                            tr("At start: on")
                        } else {
                            tr("At start: off")
                        })
                        .small()
                        .selected(at_start)
                        .tooltip(tr("Also look for a newer version each time Wisp starts"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_update_at_start(!at_start, window, cx)
                        })),
                );
                if let (true, Some(settings::Update::Found(found))) = (available, &self.update) {
                    let url = found.url.clone();
                    actions = actions.child(
                        Button::new("update-open")
                            .label(tr("Open releases"))
                            .small()
                            .primary()
                            .tooltip(tr("Open the release page in your browser"))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                    );
                }
                setting(tr("Updates"), note, actions, cx)
            })
            .child(setting(
                tr("When Wisp starts"),
                tr(
                    "Open on the home page with every project, or on the conversation that \
                 was open when you last used it.",
                ),
                starts,
                cx,
            ))
    }

    /// Reads which service keys are saved. Each keeps its field across
    /// reloads; what is in a field is only what was just typed.
    fn load_credentials(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mine = self.backend.custom_credentials();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(mine).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(rows) => this.custom_credentials = rows,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        let reply = self.backend.credentials();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(rows) => {
                        let mut fields: Vec<_> = std::mem::take(&mut this.credentials);
                        this.credentials = rows
                            .into_iter()
                            .map(|row| {
                                let kept = fields.iter().position(|(old, _)| old.id == row.id);
                                let field = match kept {
                                    Some(index) => fields.swap_remove(index).1,
                                    None => {
                                        cx.new(|cx| InputState::new(window, cx).masked(row.secret))
                                    }
                                };
                                let placeholder =
                                    settings::credential_placeholder(row.saved, row.secret);
                                field.update(cx, |input, cx| {
                                    input.set_placeholder(placeholder, window, cx)
                                });
                                (row, field)
                            })
                            .collect();
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves what is typed for one credential, or with `forget` removes it.
    fn save_credential(
        &mut self,
        id: &'static str,
        forget: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, field)) = self.credentials.iter().find(|(row, _)| row.id == id) else {
            return;
        };
        let field = field.clone();
        let value = if forget {
            String::new()
        } else {
            field.read(cx).value().trim().to_string()
        };
        if !forget && value.is_empty() {
            return;
        }
        self.settings_error = None;
        let reply = self.backend.set_credential(id, value);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    // The typed value has done its work; it is not kept on screen.
                    Ok(()) => field.update(cx, |input, cx| input.set_value("", window, cx)),
                    Err(error) => this.settings_error = Some(error),
                }
                this.load_credentials(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Adds the credential of your own the fields describe; on success the
    /// fields are emptied, the value first.
    fn add_custom_credential(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let [name, env_var, value] = [0, 1, 2].map(|at| {
            self.custom_credential_fields[at]
                .read(cx)
                .value()
                .to_string()
        });
        self.settings_error = None;
        let reply = self.backend.add_custom_credential(name, env_var, value);
        self.receive_custom_credentials(reply, true, window, cx);
    }

    fn remove_custom_credential(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        let reply = self.backend.remove_custom_credential(id);
        self.receive_custom_credentials(reply, false, window, cx);
    }

    fn receive_custom_credentials(
        &mut self,
        reply: crate::backend::Reply<Vec<wisp_server::desktop::CustomCredential>>,
        clear: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(rows) => {
                        this.custom_credentials = rows;
                        if clear {
                            for field in &this.custom_credential_fields {
                                field.update(cx, |input, cx| input.set_value("", window, cx));
                            }
                        }
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Credentials of your own: what is saved, and a form to add one.
    fn custom_credential_section(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut section = div()
            .v_flex()
            .gap_2()
            .pt_4()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(tr("Your own")))
            .child(div().w(px(NOTE_WIDTH + 80.)).text_xs().text_color(muted).child(tr(
                "Any other key, given to the code the model runs under the environment variable you name. Adding one with a variable that is already here replaces its value.",
            )));
        for credential in &self.custom_credentials {
            let id = credential.id.clone();
            section = section.child(
                div()
                    .h_flex()
                    .gap_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .w(px(180.))
                            .truncate()
                            .text_sm()
                            .child(credential.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(muted)
                            .child(credential.env_var.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(if credential.saved {
                                tr("Saved")
                            } else {
                                tr("Not set")
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("custom-credential-remove-{id}")))
                            .label(tr("Remove"))
                            .small()
                            .ghost()
                            .tooltip(tr("Remove it and its value from the keychain"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.remove_custom_credential(id.clone(), window, cx)
                            })),
                    ),
            );
        }
        let ready = self
            .custom_credential_fields
            .iter()
            .all(|field| !field.read(cx).value().trim().is_empty());
        section.child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    div()
                        .w(px(160.))
                        .child(Input::new(&self.custom_credential_fields[0]).small()),
                )
                .child(
                    div()
                        .w(px(160.))
                        .child(Input::new(&self.custom_credential_fields[1]).small()),
                )
                .child(
                    div()
                        .w(px(160.))
                        .child(Input::new(&self.custom_credential_fields[2]).small()),
                )
                .child(
                    Button::new("custom-credential-add")
                        .label(tr("Add"))
                        .small()
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_custom_credential(window, cx)
                        })),
                ),
        )
    }

    pub(super) fn render_credential_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut page = self.settings_page(cx).child(
            div()
                .w(px(NOTE_WIDTH + 80.))
                .pb_2()
                .text_xs()
                .text_color(muted)
                .child(tr(
                    "Keys for the services the connectors read from. They are kept in the \
                     system keychain, never in the project database, and are not shown again \
                     once saved. They are used from the next turn.",
                )),
        );
        if self.credentials.is_empty() {
            page = page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        }
        for (row, field) in &self.credentials {
            let id = row.id;
            // Save does nothing while the field is empty; an empty value is
            // what Forget is for.
            let mut buttons = div().h_flex().gap_2().child(
                Button::new(SharedString::from(format!("credential-save-{id}")))
                    .label(tr("Save"))
                    .small()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_credential(id, false, window, cx)
                    })),
            );
            if row.saved {
                buttons = buttons.child(
                    Button::new(SharedString::from(format!("credential-forget-{id}")))
                        .label(tr("Forget"))
                        .small()
                        .ghost()
                        .tooltip(tr("Remove it from the keychain"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.save_credential(id, true, window, cx)
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
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(crate::i18n::host(row.label)),
                            )
                            .child(div().text_xs().text_color(muted).child(if row.saved {
                                tr("Saved")
                            } else {
                                tr("Not set")
                            })),
                    )
                    .child(
                        div()
                            .w(px(NOTE_WIDTH + 80.))
                            .text_xs()
                            .text_color(muted)
                            .child(crate::i18n::host(row.note)),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(div().w(px(360.)).child(Input::new(field).small()))
                            .child(buttons),
                    ),
            );
        }
        page.child(self.custom_credential_section(cx))
    }

    pub(super) fn settings_page(&self, cx: &App) -> Div {
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
                    .child(tr("Loading…")),
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
            tr("Steps per turn"),
            crate::i18n::fill(
                tr(
                    "How many times the model may call tools or answer within one turn, from {} \
                 to {}. At the limit it stops and says where it got to.",
                ),
                &[&STEPS.start(), &STEPS.end()],
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
                        .label(tr("Save"))
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| this.save_steps(window, cx))),
                ),
            cx,
        ))
        .child(setting(
            tr("Compact automatically"),
            tr(
                "When the context fills up, older turns are summarised so the conversation \
             can go on. Off, a long conversation ends when the model's window is full.",
            ),
            Button::new("auto-compact")
                .label(if defaults.auto_compact {
                    tr("On")
                } else {
                    tr("Off")
                })
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
        .child({
            let limit = defaults.auto_continue_limit;
            let step = |id: &'static str, name: &'static str, tip: &'static str, up: bool| {
                let next = settings::step_continues(limit, up);
                icon_button(id, name, tip)
                    .disabled(!defaults.auto_continue || next == limit)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_turn_defaults(
                            TurnDefaults {
                                auto_continue_limit: next,
                                ..defaults
                            },
                            window,
                            cx,
                        )
                    }))
            };
            setting(
                tr("Continue cut-off answers"),
                tr(
                    "When an answer stops at the model's output limit, the turn carries on by \
                 itself, at most this many times. Off, a cut answer ends the turn.",
                ),
                div()
                    .h_flex()
                    .gap_1()
                    .child(
                        Button::new("auto-continue")
                            .label(if defaults.auto_continue {
                                tr("On")
                            } else {
                                tr("Off")
                            })
                            .small()
                            .selected(defaults.auto_continue)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.save_turn_defaults(
                                    TurnDefaults {
                                        auto_continue: !defaults.auto_continue,
                                        ..defaults
                                    },
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(step("continue-down", "minus", tr("Fewer times"), false))
                    .child(
                        div()
                            .w(px(36.))
                            .text_center()
                            .text_sm()
                            .child(limit.to_string()),
                    )
                    .child(step("continue-up", "plus", tr("More times"), true)),
                cx,
            )
        })
        .child(setting(
            tr("Compact at"),
            crate::i18n::fill(
                tr(
                    "How full the context is when that happens, from {}% to {}%. Lower \
                 compacts sooner and keeps more room for long tool output.",
                ),
                &[&low, &high],
            ),
            div()
                .h_flex()
                .gap_1()
                .child(step("compact-down", "minus", tr("Compact sooner"), false))
                .child(
                    div()
                        .w(px(48.))
                        .text_center()
                        .text_sm()
                        .child(format!("{percent}%")),
                )
                .child(step("compact-up", "plus", tr("Compact later"), true)),
            cx,
        ))
        .child(setting(
            tr("Suggest what to ask next"),
            tr(
                "After an answer the model is asked once more, briefly, for up to three \
             questions you could ask next. They show above the composer. Off, that extra \
             request is not made.",
            ),
            Button::new("follow-ups")
                .label(if defaults.follow_ups {
                    tr("On")
                } else {
                    tr("Off")
                })
                .small()
                .selected(defaults.follow_ups)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            follow_ups: !defaults.follow_ups,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                })),
            cx,
        ))
        .child(setting(
            tr("Review turns"),
            tr(
                "When a turn that used a tool, or said a good deal, ends, a reviewer checks its \
             claims against the tool output and the result shows above the composer. This \
             is what conversations start with; each can be switched in its composer.",
            ),
            Button::new("auto-review-default")
                .label(if defaults.auto_review {
                    tr("On")
                } else {
                    tr("Off")
                })
                .small()
                .selected(defaults.auto_review)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            auto_review: !defaults.auto_review,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                })),
            cx,
        ))
        .children(
            self.failure_analysis
                .map(|failures| self.failure_analysis_setting(failures, cx)),
        )
        .child(setting(
            tr("Reviewer model"),
            tr(
                "The model that reviews, for automatic reviews and those you ask for. A second \
             model catches more than a model checking itself.",
            ),
            self.review_model_choices(cx),
            cx,
        ))
        .child(setting(
            tr("Sub-agent model"),
            tr(
                "The model that helpers the main model sends off to explore work on. A \
                 cheaper, faster model here keeps long searches off the conversation's model.",
            ),
            self.subagent_model_choices(cx),
            cx,
        ))
        .child(setting(
            tr("Picture model"),
            tr(
                "When the conversation's model cannot see, this one looks at the pictures you \
                 attach and tells it what they show. Only models that can see are listed.",
            ),
            self.picture_model_choices(cx),
            cx,
        ))
    }

    /// The Conversation page's row for drafting a memory when tools fail:
    /// on or off, the share of failed calls, and how many at least.
    fn failure_analysis_setting(
        &self,
        failures: wisp_server::desktop::AutoFailureAnalysisSettings,
        cx: &mut Context<Self>,
    ) -> Div {
        let value = |text: String| div().w(px(44.)).text_center().text_sm().child(text);
        let controls = div()
            .h_flex()
            .gap_1()
            .child(
                Button::new("failure-analysis")
                    .label(if failures.enabled {
                        tr("On")
                    } else {
                        tr("Off")
                    })
                    .small()
                    .selected(failures.enabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let next = wisp_server::desktop::AutoFailureAnalysisSettings {
                            enabled: !failures.enabled,
                            ..failures
                        };
                        this.save_failure_analysis(next, window, cx)
                    })),
            )
            .child(Self::failure_step(
                "failure-rate-down",
                "minus",
                tr("A smaller share"),
                failures,
                true,
                false,
                cx,
            ))
            .child(value(format!("{}%", failures.failure_rate_threshold)))
            .child(Self::failure_step(
                "failure-rate-up",
                "plus",
                tr("A larger share"),
                failures,
                true,
                true,
                cx,
            ))
            .child(Self::failure_step(
                "failure-count-down",
                "minus",
                tr("Fewer failures"),
                failures,
                false,
                false,
                cx,
            ))
            .child(value(failures.minimum_failures.to_string()))
            .child(Self::failure_step(
                "failure-count-up",
                "plus",
                tr("More failures"),
                failures,
                false,
                true,
                cx,
            ));
        setting(
            tr("Draft a memory when tools fail"),
            tr(
                "When this share of a turn's tool calls failed, and at least this many, the \
             model drafts what to remember about why, for you to confirm. A turn that asks \
             Wisp to remember something has a draft either way.",
            ),
            controls,
            cx,
        )
    }

    /// One step of a failure threshold, off while the drafting is off.
    fn failure_step(
        id: &'static str,
        name: &'static str,
        tip: &'static str,
        failures: wisp_server::desktop::AutoFailureAnalysisSettings,
        rate: bool,
        up: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let next = settings::step_failures(failures, rate, up);
        icon_button(id, name, tip)
            .disabled(!failures.enabled || next == failures)
            .on_click(
                cx.listener(move |this, _, window, cx| {
                    this.save_failure_analysis(next, window, cx)
                }),
            )
    }

    /// Saves when a turn whose tools failed often has a memory drafted.
    fn save_failure_analysis(
        &mut self,
        next: wisp_server::desktop::AutoFailureAnalysisSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.failure_analysis = Some(next);
        let reply = self.backend.set_failure_analysis(next);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(saved) => this.failure_analysis = Some(saved),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The conversation's own model, or one of the saved ones.
    fn subagent_model_choices(&self, cx: &mut Context<Self>) -> Div {
        let current = self.subagent_model.as_str();
        let profiles = self
            .settings
            .current()
            .map(|models| models.profiles.as_slice())
            .unwrap_or_default();
        // A profile removed since reads as the conversation's own.
        let own = !profiles.iter().any(|profile| profile.id == current);
        let mut choices = div().h_flex().flex_wrap().gap_1().justify_end().child(
            Button::new("subagent-model-own")
                .label(tr("The conversation's"))
                .small()
                .selected(own)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.save_subagent_model(String::new(), window, cx)
                })),
        );
        for profile in profiles {
            let id = profile.id.clone();
            choices = choices.child(
                Button::new(SharedString::from(format!("subagent-model-{}", profile.id)))
                    .label(profile.name.clone())
                    .small()
                    .selected(profile.id == current)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_subagent_model(id.clone(), window, cx)
                    })),
            );
        }
        choices
    }

    /// The first saved model that can see, or one of those that can.
    fn picture_model_choices(&self, cx: &mut Context<Self>) -> Div {
        let current = self.picture_model.as_str();
        let (profiles, first) = crate::settings::picture_choices(self.settings.current(), current);
        let mut choices = div().h_flex().flex_wrap().gap_1().justify_end().child(
            Button::new("picture-model-first")
                .label(if profiles.is_empty() {
                    tr("None can see")
                } else {
                    tr("The first that can see")
                })
                .small()
                .selected(first)
                .disabled(profiles.is_empty())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.save_picture_model(String::new(), window, cx)
                })),
        );
        for profile in profiles {
            let id = profile.id.clone();
            choices = choices.child(
                Button::new(SharedString::from(format!("picture-model-{}", profile.id)))
                    .label(profile.name.clone())
                    .small()
                    .selected(profile.id == current)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_picture_model(id.clone(), window, cx)
                    })),
            );
        }
        choices
    }

    /// The conversation's own model, or one of the saved ones.
    fn review_model_choices(&self, cx: &mut Context<Self>) -> Div {
        let current = self.review_model.as_str();
        let profiles = self
            .settings
            .current()
            .map(|models| models.profiles.as_slice())
            .unwrap_or_default();
        // A profile removed since reads as the conversation's own.
        let own = !profiles.iter().any(|profile| profile.id == current);
        let mut choices = div().h_flex().flex_wrap().gap_1().justify_end().child(
            Button::new("review-model-own")
                .label(tr("The conversation's"))
                .small()
                .selected(own)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.save_review_model(String::new(), window, cx)
                })),
        );
        for profile in profiles {
            let id = profile.id.clone();
            choices = choices.child(
                Button::new(SharedString::from(format!("review-model-{}", profile.id)))
                    .label(profile.name.clone())
                    .small()
                    .selected(profile.id == current)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_review_model(id.clone(), window, cx)
                    })),
            );
        }
        choices
    }

    pub(super) fn render_permission_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(defaults) = self.turn_defaults else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
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
            tr("New conversations start in"),
            tr(
                "Ask first: tools that change files or run commands wait for your answer. \
             Full access: they run at once. A dangerous shell command asks either way. \
             Conversations you already have keep their own choice.",
            ),
            div()
                .h_flex()
                .gap_1()
                .child(mode("default-ask", tr("Ask first"), false))
                .child(mode("default-full", tr("Full access"), true)),
            cx,
        ));
        page = page.child(self.approval_settings(cx));
        page = page.child(
            div()
                .v_flex()
                .gap_1()
                .pt_4()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(tr("Always allowed")),
                )
                .child(
                    div()
                        .w(px(NOTE_WIDTH))
                        .text_xs()
                        .text_color(muted)
                        .child(tr(
                            "Tools you answered \"Always in this project\" for. They run without \
                     asking in every conversation of that project until you take them back.",
                        )),
                ),
        );
        if !self.global_grants.is_empty() {
            let mut tools = div().h_flex().flex_wrap().gap_2();
            for tool in &self.global_grants {
                let name = tool.clone();
                tools = tools.child(
                    Button::new(SharedString::from(format!("revoke-everywhere-{tool}")))
                        .icon(glyph("close"))
                        .label(tool.clone())
                        .small()
                        .tooltip(tr("Ask again before this tool runs"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.revoke_global_grant(name.clone(), window, cx)
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
                    .child(div().text_sm().child(tr("Every project")))
                    .child(tools),
            );
        }
        if self.grants.is_empty() && self.global_grants.is_empty() {
            page = page.child(
                div()
                    .pt_3()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("No tool is always allowed.")),
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
                    .tooltip(tr("Ask again before this tool runs"))
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
