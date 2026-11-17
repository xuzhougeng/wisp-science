//! The Settings pages besides Models: how turns run (Conversation) and what
//! runs without asking (Permissions). Each change is saved as it is made and
//! applies from the next turn.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::InputState,
};
use wisp_server::desktop::TurnDefaults;

use super::{
    Workspace,
    shell::{error_banner, glyph, icon_button},
    surfaces::{self, switch},
};
use crate::{
    assets::icon,
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
/// The palette of each kind, and the text's sizes and faces.
const LIGHT_PALETTE: &str = "light_palette";
const DARK_PALETTE: &str = "dark_palette";
const TEXT_SIZE: &str = "text_size";
const CODE_SIZE: &str = "code_size";
const TEXT_FONT: &str = "text_font";
const CODE_FONT: &str = "code_font";
const NOTIFY: &str = "notifications";
const NOTIFY_SOUND: &str = "notification_sound";
/// Which of the system's sounds is played.
const NOTIFY_TONE: &str = "notification_tone";
const LAST_OPEN: &str = "last_open";

/// Set once the check at start has been made, so a second window does not
/// make it again.
static CHECKED_AT_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Steps one turn may take; the host's range.
const STEPS: std::ops::RangeInclusive<usize> = 1..=1_000;

/// A setting, the one row shape of every settings page: its name in ink
/// and, where the name alone is not enough, one faded note under it, on
/// the left; its control on the right, centred on the row; a hairline
/// under it. Name and note are the same, regular weight.
pub(super) fn setting(
    title: &'static str,
    note: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    let note: SharedString = note.into();
    item_row(
        SharedString::from(tr(title)),
        (!note.trim().is_empty()).then(|| surfaces::settings_note(note, cx).into_any_element()),
        control,
        cx,
    )
}

/// `setting` whose name is followed by a `help` icon: the note stays one
/// short line (or none), the rest of what it says is the icon's tooltip.
pub(super) fn explained_setting(
    title: &'static str,
    note: impl Into<SharedString>,
    explained: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    let note: SharedString = note.into();
    item_row(
        div()
            .h_flex()
            .items_center()
            .gap_1()
            .child(tr(title))
            .child(surfaces::help(
                SharedString::from(format!("help-{title}")),
                explained,
                cx,
            )),
        (!note.trim().is_empty()).then(|| surfaces::settings_note(note, cx).into_any_element()),
        control,
        cx,
    )
}

/// A setting whose control needs the column's width (a long text, a group
/// of chips): its name and note on top, the control under them, a hairline
/// under it all.
pub(super) fn stacked_setting(
    title: &'static str,
    note: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    let note: SharedString = note.into();
    div()
        .v_flex()
        .gap_2()
        .py_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .v_flex()
                .gap_0p5()
                .child(div().text_sm().child(title))
                .when(!note.trim().is_empty(), |text| {
                    text.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(note),
                    )
                }),
        )
        .child(control)
}

/// The row shape of `setting` for an item of a list whose name is data (a
/// skill, a connector): its name in ink, what is under it faded, its
/// controls at the right.
fn item_row(
    name: impl IntoElement,
    under: Option<AnyElement>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    div()
        .h_flex()
        .items_center()
        .justify_between()
        .gap_6()
        .py_3()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().child(name))
                .children(under),
        )
        .child(div().flex_shrink_0().max_w(relative(0.6)).child(control))
}

/// A count with a step down and a step up in one segmented box: `minus` |
/// the value as a fact | `plus`. A step that cannot be taken now has no
/// handler and is faded.
fn count_stepper(
    id: &'static str,
    value: String,
    down: (
        &'static str,
        Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    ),
    up: (
        &'static str,
        Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    ),
    cx: &App,
) -> Div {
    let end = |part: &'static str,
               icon_name: &'static str,
               (tip, run): (
        &'static str,
        Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    )| {
        let live = run.is_some();
        surfaces::pressable(SharedString::from(format!("{id}-{part}")))
            .group(surfaces::ICON_HOVER)
            .h_flex()
            .items_center()
            .justify_center()
            .w(px(surfaces::ICON_BUTTON))
            .h(px(26.))
            .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tip).build(window, cx))
            .when(!live, |end| end.opacity(0.4))
            .when_some(run, |end, run| {
                let hover = cx.theme().list_hover;
                end.cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(move |event, window, cx| run(event, window, cx))
            })
            .child(surfaces::sized_icon(
                icon_name,
                surfaces::IconSize::Inline,
                surfaces::IconTone::Faded,
                cx,
            ))
            .into_any_element()
    };
    surfaces::segmented(
        [
            end("down", "minus", down),
            div()
                .h_flex()
                .items_center()
                .justify_center()
                .min_w(px(44.))
                .px_2()
                .h(px(26.))
                .child(surfaces::fact(value, cx).text_color(cx.theme().foreground))
                .into_any_element(),
            end("up", "plus", up),
        ],
        cx,
    )
}

/// A control with the detail a setting's note leaves out as its tooltip.
fn tipped(id: &'static str, tip: &'static str, control: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tip).build(window, cx))
        .child(control)
}

/// A wrapping group of model chips, kept to the right of its row.
fn model_chips() -> Div {
    div()
        .h_flex()
        .flex_wrap()
        .justify_end()
        .gap_1()
        .max_w(px(380.))
}

/// A search field across the column: white, a search icon before it.
fn search_field(state: &Entity<InputState>, cx: &App) -> Div {
    div().py_3().w_full().child(
        surfaces::input(state, cx)
            .small()
            .prefix(surfaces::sized_icon(
                "search",
                surfaces::IconSize::Inline,
                surfaces::IconTone::Faded,
                cx,
            )),
    )
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
            Section::General => self.load_storage_choices(window, cx),
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
        let mut section = div().v_flex();
        let Some((project, approvals)) = &self.approvals else {
            return section.child(setting(
                tr("Tool approval mode"),
                tr("Open a project to choose how its tools are approved."),
                div(),
                cx,
            ));
        };
        let name = self
            .projects
            .iter()
            .find(|row| &row.id == project)
            .map_or_else(|| project.clone(), |row| row.name.clone());
        let modes = surfaces::segmented(
            settings::APPROVAL_SCOPES.map(|(scope, label, _)| {
                surfaces::plain_segment(
                    SharedString::from(format!("approval-scope-{label}")),
                    tr(label),
                    approvals.scope == scope,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_approval_scope(scope, window, cx)
                }))
                .into_any_element()
            }),
            cx,
        );
        let note = settings::approval_note(approvals.scope);
        section = section.child(setting(
            tr("Tool approval mode"),
            crate::i18n::fill(tr("For {}, from the next turn."), &[&name]),
            div()
                .id("approval-scope")
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(note.clone()).build(window, cx)
                })
                .child(modes),
            cx,
        ));
        if !approvals.rules.is_empty() {
            let mut rows = Vec::new();
            for (tool, rule) in &approvals.rules {
                let name = tool.clone();
                let said = match rule {
                    wisp_server::desktop::ToolRule::Ask => tr("Ask first"),
                    wisp_server::desktop::ToolRule::Deny => tr("Deny"),
                };
                rows.push(
                    surfaces::record_row(
                        SharedString::from(format!("tool-rule-{tool}")),
                        tool.clone(),
                    )
                    .icon("wrench")
                    .mono()
                    .suffix(said)
                    .action(
                        surfaces::icon_action(
                            SharedString::from(format!("tool-rule-clear-{tool}")),
                            "close",
                            tr("Allow it again"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.save_tool_rule(name.clone(), None, window, cx)
                            },
                        )),
                    ),
                );
            }
            section = section
                .child(surfaces::settings_heading(
                    tr("Tools with a rule of their own, set on the Connections page:"),
                    cx,
                ))
                .child(surfaces::record_group(rows, cx));
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
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(surfaces::settings_note(tr("Skip approvals"), cx))
                .child(
                    switch(
                        SharedString::from(format!("connector-skip-{connector}")),
                        on,
                        tr("Its tools run without asking, whatever their rules"),
                    )
                    .on_change(cx.listener(
                        move |this, value: &bool, window, cx| {
                            this.save_connector_skip(domain.clone(), *value, window, cx)
                        },
                    )),
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
        let Some((_, approvals)) = &self.approvals else {
            return surfaces::fact(tools.join("  "), cx);
        };
        // Skipped, the rules do not apply.
        let skipped = approvals.skip.contains(connector);
        let mut list = div().v_flex().gap_1();
        for tool in tools {
            let current = approvals.rules.get(tool).copied();
            let rules = surfaces::segmented(
                settings::TOOL_RULES.map(|(rule, label)| {
                    let name = tool.clone();
                    surfaces::plain_segment(
                        SharedString::from(format!("rule-{tool}-{label}")),
                        tr(label),
                        current == rule,
                        cx,
                    )
                    .when(!skipped, |segment| {
                        segment.on_click(cx.listener(move |this, _, window, cx| {
                            this.save_tool_rule(name.clone(), rule, window, cx)
                        }))
                    })
                    .into_any_element()
                }),
                cx,
            )
            .when(skipped, |rules| rules.opacity(0.5));
            list = list.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(surfaces::read_only(tool.clone(), cx).flex_1())
                    .child(rules),
            );
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
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().child(settings::skills_summary(skills)))
                            .child(if self.state.project.is_some() {
                                surfaces::settings_note_help(
                                    tr("The open project's skills and those for every project."),
                                    tr(
                                        "The open project's skills and the ones for every project. \
                                     A skill that is off is not offered to the model.",
                                    ),
                                    cx,
                                )
                            } else {
                                surfaces::settings_note_help(
                                    tr("Skills for every project."),
                                    tr("Skills for every project. Open a project to see its own \
                                     as well. A skill that is off is not offered to the model."),
                                    cx,
                                )
                            }),
                    )
                    .child(
                        surfaces::icon_action(
                            "skills-reload",
                            "refresh",
                            tr("Read the skill folders again"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.load_skills(window, cx))),
                    ),
            )
            .child(search_field(&self.skills_search, cx));
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
        let mut list = div().v_flex().border_t_1().border_color(cx.theme().border);
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
            let under = div()
                .v_flex()
                .gap_0p5()
                .child(surfaces::settings_note(row.description.clone(), cx))
                .child(surfaces::fact(about, cx));
            list = list.child(
                item_row(
                    SharedString::from(row.name.clone()),
                    Some(under.into_any_element()),
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            surfaces::icon_action(
                                SharedString::from(format!("skill-tags-{}", row.name)),
                                "edit",
                                tr("Give it your own tags in place of the ones it came with"),
                                if editing {
                                    surfaces::IconTone::Ink
                                } else {
                                    surfaces::IconTone::Faded
                                },
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.edit_skill_tags(tagged.clone(), &current, window, cx)
                                },
                            )),
                        )
                        .child(
                            switch(
                                SharedString::from(format!("skill-{}", row.name)),
                                enabled,
                                tr("Use this skill"),
                            )
                            .on_change(cx.listener(
                                move |this, _, window, cx| {
                                    this.set_skill_enabled(name.clone(), !enabled, window, cx)
                                },
                            )),
                        ),
                    cx,
                )
                .when(!enabled, |row| row.text_color(muted)),
            );
            if editing {
                // The skill's tags, under it, as a row of the same shape.
                list = list.child(setting(
                    tr("Tags"),
                    "",
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .w(px(240.))
                                .child(surfaces::input(&self.skill_tags_input, cx).small()),
                        )
                        .child(
                            surfaces::outlined_action("skill-tags-save", "check", tr("Save"), cx)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_skill_tags(window, cx)
                                })),
                        )
                        .child(
                            surfaces::icon_action(
                                "skill-tags-cancel",
                                "close",
                                tr("Cancel"),
                                surfaces::IconTone::Faded,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.skill_tags_editing = None;
                                this.layers.close(Layer::SkillTags);
                                cx.notify();
                            })),
                        ),
                    cx,
                ));
            }
        }
        page.child(list).child(self.skill_store_section(cx))
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
            .child(surfaces::settings_heading(
                settings::connectors_summary(connectors),
                cx,
            ))
            .child(surfaces::settings_note_help(
                tr("Literature and biology sources that ship with Wisp."),
                tr(
                    "Literature and biology sources that ship with Wisp. The model can \
                    only read through them. A connector that is off gives it no tools.",
                ),
                cx,
            ))
            .child(search_field(&self.connectors_search, cx));
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
        let mut list = div().v_flex().border_t_1().border_color(cx.theme().border);
        for row in shown {
            let enabled = row.enabled;
            let open = self.open_connector.as_deref() == Some(row.domain.as_str());
            let (toggling, unfolding) = (row.domain.clone(), row.domain.clone());
            let controls = div()
                .h_flex()
                .items_center()
                .gap_1()
                .child(
                    surfaces::labelled_action(
                        SharedString::from(format!("connector-tools-{}", row.domain)),
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        settings::tool_count(row.tools.len()),
                        surfaces::IconSize::Inline,
                        if open {
                            surfaces::IconTone::Ink
                        } else {
                            surfaces::IconTone::Faded
                        },
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_connector = (this.open_connector.as_deref()
                            != Some(unfolding.as_str()))
                        .then(|| unfolding.clone());
                        cx.notify();
                    })),
                )
                .child(
                    switch(
                        SharedString::from(format!("connector-{}", row.domain)),
                        enabled,
                        tr("Use this connector"),
                    )
                    .on_change(cx.listener(move |this, _, window, cx| {
                        this.set_connector_enabled(toggling.clone(), !enabled, window, cx)
                    })),
                );
            list = list.child(
                item_row(
                    SharedString::from(row.domain.clone()),
                    Some(surfaces::settings_note(row.description.clone(), cx).into_any_element()),
                    controls,
                    cx,
                )
                .when(!enabled, |row| row.text_color(muted)),
            );
            if open {
                // What the connector opens to, under it: whether its tools
                // ask, each tool's rule, and where it reads from.
                let mut detail = div()
                    .v_flex()
                    .gap_2()
                    .py_2()
                    .pl_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(self.connector_skip(&row.domain, cx))
                    .child(self.connector_tool_rules(&row.domain, &row.tools, cx));
                for (index, (label, url)) in row.links.iter().enumerate() {
                    let target = url.clone();
                    detail = detail.child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .child(surfaces::settings_note(label.clone(), cx).flex_none())
                            .child(
                                surfaces::fact(url.clone(), cx)
                                    .flex_1()
                                    .min_w_0()
                                    .truncate(),
                            )
                            .child(
                                surfaces::icon_action(
                                    SharedString::from(format!(
                                        "connector-link-{}-{index}",
                                        row.domain
                                    )),
                                    "external-link",
                                    tr("Open in your browser"),
                                    surfaces::IconTone::Faded,
                                    cx,
                                )
                                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&target))),
                            ),
                    );
                }
                list = list.child(detail);
            }
        }
        page.child(list)
    }

    /// Reads the choices of the General page and acts on them: wears the
    /// chosen palette and, if asked to, reopens the conversation that was
    /// open last time.
    pub(super) fn load_general(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (banner, sound, tone) = (
            self.backend.preference(NOTIFY),
            self.backend.preference(NOTIFY_SOUND),
            self.backend.preference(NOTIFY_TONE),
        );
        cx.spawn_in(window, async move |this, cx| {
            let saved = |reply| async move { receive(reply).await.ok().flatten() };
            let (banner, sound, tone) =
                (saved(banner).await, saved(sound).await, saved(tone).await);
            let _ = this.update(cx, |this, cx| {
                this.notify =
                    crate::notify::Choice::from_saved(banner.as_deref(), sound.as_deref());
                this.notify_tone =
                    crate::notify::sound_of(crate::notify::System::current(), tone.as_deref());
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
        self.load_privacy(window, cx);
        let look = [
            LIGHT_PALETTE,
            DARK_PALETTE,
            TEXT_SIZE,
            CODE_SIZE,
            TEXT_FONT,
            CODE_FONT,
        ]
        .map(|name| self.backend.preference(name));
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
            let mut kept = Vec::new();
            for reply in look {
                kept.push(saved(reply).await.unwrap_or_default());
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.look = crate::theme::Look {
                    light: crate::theme::palette_of(false, &kept[0]),
                    dark: crate::theme::palette_of(true, &kept[1]),
                    text_size: kept[2].parse().unwrap_or(0.),
                    code_size: kept[3].parse().unwrap_or(0.),
                    text_font: kept[4].clone(),
                    code_font: kept[5].clone(),
                };
                for (field, value) in this.font_fields.iter().zip([&kept[4], &kept[5]]) {
                    field.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
                }
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
                crate::theme::restyle(&this.look, this.appearance, window, cx);
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

    /// A newer version, found at start or when asked in Settings: a card at
    /// the foot of the sidebar (or on the home page, which has none) that
    /// names it and leads to the release page. Not now puts it away until
    /// the next start; Don't check at start stops looking for one.
    pub(super) fn update_card(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let found = self.update_banner.as_ref()?;
        let line = settings::update_card_line(Some(found))?;
        let url = found.url.clone();
        Some(
            surfaces::card("update-card", cx)
                .flex_shrink_0()
                .v_flex()
                .gap_1()
                .p_2()
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(icon("download").text_color(cx.theme().primary))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(tr("Update available")),
                        )
                        .child(
                            icon_button("update-card-close", "close", tr("Not now"))
                                .xsmall()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.update_banner = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(line),
                )
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .gap_1()
                        .child(
                            Button::new("update-card-open")
                                .icon(glyph("external-link"))
                                .label(tr("Open releases"))
                                .xsmall()
                                .outline()
                                .tooltip(tr("Open the release page in your browser"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.open_url(&url);
                                    this.update_banner = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("update-card-never")
                                .icon(glyph("ban"))
                                .label(tr("Don't check at start"))
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_update_at_start(false, window, cx)
                                })),
                        ),
                ),
        )
    }

    /// Asks the release page whether a newer version has been published.
    pub(super) fn check_for_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update = Some(settings::Update::Checking);
        let reply = self.backend.check_for_update();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                let update = match result {
                    Ok(found) => settings::Update::Found(found),
                    Err(problem) => settings::Update::Failed(problem),
                };
                // A newer version found when asked is offered in the
                // sidebar too, as the one found at start is.
                if let Some(found) = settings::newer_release(&update) {
                    this.update_banner = Some(found.clone());
                }
                this.update = Some(update);
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

    /// Reads the workspace folder chosen for the next start and whether new
    /// projects keep their own database.
    fn load_storage_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (chosen, separate) = (
            self.backend.chosen_workspace(),
            self.backend.separate_project_storage(),
        );
        cx.spawn_in(window, async move |this, cx| {
            let (chosen, separate) = (receive(chosen).await, receive(separate).await);
            let _ = this.update_in(cx, |this, window, cx| {
                let chosen = chosen.ok().flatten().unwrap_or_default();
                this.workspace_choice
                    .update(cx, |input, cx| input.set_value(chosen, window, cx));
                match separate {
                    Ok(on) => this.separate_storage = Some(on),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Keeps the typed workspace folder for the next start.
    fn save_workspace_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.workspace_choice.read(cx).value().to_string();
        self.settings_error = None;
        self.workspace_saved = false;
        let folder = match settings::typed_workspace(&typed) {
            Ok(folder) => folder,
            Err(problem) => {
                self.settings_error = Some(tr(problem).into());
                cx.notify();
                return;
            }
        };
        let reply = self.backend.choose_workspace(folder);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.workspace_saved = true,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Asks for the workspace folder with the system's folder picker.
    fn pick_workspace_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Choose").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picker.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.workspace_choice.update(cx, |input, cx| {
                    input.set_value(folder.display().to_string(), window, cx)
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn set_separate_storage(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.set_separate_project_storage(on);
        self.separate_storage = Some(on);
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = receive(reply).await {
                let _ = this.update(cx, |this, cx| {
                    this.separate_storage = Some(!on);
                    this.settings_error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Wears light, dark or the system's appearance and keeps the choice.
    pub(super) fn set_appearance(
        &mut self,
        choice: crate::theme::Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance = choice;
        crate::theme::apply(choice, window, cx);
        self.save_general(APPEARANCE, choice.key().into(), window, cx);
    }

    /// Wears `look` at once and keeps it for the next start.
    fn change_look(
        &mut self,
        look: crate::theme::Look,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = std::mem::replace(&mut self.look, look);
        crate::theme::restyle(&self.look, self.appearance, window, cx);
        let now = self.look.clone();
        for (name, old, new) in [
            (
                LIGHT_PALETTE,
                before.light.key.to_owned(),
                now.light.key.to_owned(),
            ),
            (
                DARK_PALETTE,
                before.dark.key.to_owned(),
                now.dark.key.to_owned(),
            ),
            (
                TEXT_SIZE,
                before.text_size.to_string(),
                now.text_size.to_string(),
            ),
            (
                CODE_SIZE,
                before.code_size.to_string(),
                now.code_size.to_string(),
            ),
            (TEXT_FONT, before.text_font, now.text_font),
            (CODE_FONT, before.code_font, now.code_font),
        ] {
            if old != new {
                self.save_general(name, new, window, cx);
            }
        }
        cx.refresh_windows();
        cx.notify();
    }

    /// Makes a text size a pixel smaller or larger, starting from 14 when
    /// it was Wisp's own; `by` 0 goes back to Wisp's own.
    pub(super) fn step_size(
        &mut self,
        size: f32,
        by: f32,
        change: fn(&mut crate::theme::Look, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut look = self.look.clone();
        let from = if size == 0. { 14. } else { size };
        let next = (from + by).clamp(
            *crate::theme::TEXT_SIZES.start(),
            *crate::theme::TEXT_SIZES.end(),
        );
        change(&mut look, if by == 0. { 0. } else { next });
        self.change_look(look, window, cx)
    }

    /// One size of text, with buttons that make it smaller or larger and
    /// one that goes back to Wisp's own.
    /// A text size as a stepper: smaller, the size, larger, and back to
    /// Wisp's own.
    fn size_choice(
        &self,
        id: &'static str,
        size: f32,
        change: fn(&mut crate::theme::Look, f32),
        cx: &mut Context<Self>,
    ) -> Div {
        surfaces::stepper(
            id,
            if size == 0. {
                tr("Wisp's own").to_owned()
            } else {
                format!("{size} px")
            },
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.step_size(size, -1., change, window, cx)
            }),
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.step_size(size, 1., change, window, cx)
            }),
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.step_size(size, 0., change, window, cx)
            }),
            cx,
        )
    }

    fn palette_choice(&self, dark: bool, cx: &mut Context<Self>) -> Div {
        let chosen = if dark {
            self.look.dark
        } else {
            self.look.light
        };
        // One group of chips, the colour's dot and its name; the chosen one
        // has a teal line.
        let mut row = div()
            .h_flex()
            .flex_wrap()
            .justify_end()
            .gap_1()
            .max_w(px(360.));
        for palette in crate::theme::palettes(dark) {
            let theme = cx.theme();
            let dot = div()
                .size(px(10.))
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .bg(gpui::Rgba::try_from(palette.accent)
                    .map(Hsla::from)
                    .unwrap_or(theme.primary));
            row = row.child(
                surfaces::choice_chip(
                    SharedString::from(format!(
                        "palette-{}-{}",
                        if dark { "dark" } else { "light" },
                        palette.key
                    )),
                    palette == chosen,
                    cx,
                )
                .child(dot)
                .child(palette.name)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let mut look = this.look.clone();
                    if dark {
                        look.dark = palette;
                    } else {
                        look.light = palette;
                    }
                    this.change_look(look, window, cx)
                })),
            );
        }
        row
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
        // One of a few: a segmented control each.
        let looks = surfaces::segmented(
            crate::theme::Appearance::ALL.map(|choice| {
                surfaces::segment(
                    SharedString::from(format!("appearance-{}", choice.key())),
                    match choice {
                        crate::theme::Appearance::System => "monitor",
                        crate::theme::Appearance::Light => "sun",
                        crate::theme::Appearance::Dark => "moon",
                    },
                    tr(choice.label()),
                    self.appearance == choice,
                    cx,
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.set_appearance(choice, window, cx)),
                )
                .into_any_element()
            }),
            cx,
        );
        let languages = surfaces::segmented(
            crate::i18n::Language::ALL.map(|choice| {
                // "System" is a word of the interface; the languages are
                // named in themselves.
                surfaces::plain_segment(
                    SharedString::from(format!("language-{}", choice.key())),
                    tr(choice.label()),
                    self.language == choice,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.language = choice;
                    crate::i18n::apply(choice);
                    this.word_fields(window, cx);
                    this.save_general(LANGUAGE, choice.key().into(), window, cx);
                    cx.refresh_windows();
                }))
                .into_any_element()
            }),
            cx,
        );
        let sends = surfaces::segmented(
            settings::SendKey::ALL.map(|choice| {
                surfaces::plain_segment(
                    SharedString::from(format!("send-{}", choice.key())),
                    choice.label(),
                    self.send_key == choice,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.send_key = choice;
                    // The empty composer says which key it is now.
                    this.word_fields(window, cx);
                    this.save_general(SEND_KEY, choice.key().into(), window, cx);
                }))
                .into_any_element()
            }),
            cx,
        );
        let starts = surfaces::segmented(
            settings::Launch::ALL.map(|choice| {
                surfaces::plain_segment(
                    SharedString::from(format!("launch-{}", choice.key())),
                    choice.label(),
                    self.launch == choice,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.launch = choice;
                    this.save_general(LAUNCH, choice.key().into(), window, cx);
                }))
                .into_any_element()
            }),
            cx,
        );
        self.settings_page(cx)
            .child(setting(
                tr("Language"),
                "",
                tipped(
                    "language-tip",
                    tr(
                        "The language of the interface. System follows your computer's language: \
                 Chinese when it is set to Chinese, English otherwise. What the model \
                 writes is not affected.",
                    ),
                    languages,
                ),
                cx,
            ))
            .child(setting(
                tr("Appearance"),
                "",
                tipped(
                    "appearance-tip",
                    tr("System follows your computer's light or dark setting and changes with it."),
                    looks,
                ),
                cx,
            ))
            .child(setting(
                tr("Light palette"),
                "",
                tipped(
                    "light-palette-tip",
                    tr("The colours worn in light appearance. Wisp Paper is Wisp's own."),
                    self.palette_choice(false, cx),
                ),
                cx,
            ))
            .child(setting(
                tr("Dark palette"),
                "",
                tipped(
                    "dark-palette-tip",
                    tr("The colours worn in dark appearance. Wisp Charcoal is Wisp's own."),
                    self.palette_choice(true, cx),
                ),
                cx,
            ))
            .child(setting(
                tr("Text size"),
                "",
                tipped(
                    "text-size-tip",
                    tr("The size of the interface's text, from 10 to 24 pixels."),
                    self.size_choice(
                        "text-size",
                        self.look.text_size,
                        |look, size| look.text_size = size,
                        cx,
                    ),
                ),
                cx,
            ))
            .child(setting(
                tr("Code size"),
                "",
                tipped(
                    "code-size-tip",
                    tr("The size of code and other fixed-width text."),
                    self.size_choice(
                        "code-size",
                        self.look.code_size,
                        |look, size| look.code_size = size,
                        cx,
                    ),
                ),
                cx,
            ))
            // Two rows of the one shape, each its field at the right edge;
            // Apply under them at the same edge.
            .child(explained_setting(
                tr("Interface font"),
                "",
                tr("A font installed on this computer for the interface, and one for code, \
                     by name. Empty keeps Wisp's own."),
                div()
                    .w(px(220.))
                    .child(surfaces::input(&self.font_fields[0], cx).small()),
                cx,
            ))
            .child(setting(
                tr("Code font"),
                "",
                div()
                    .w(px(220.))
                    .child(surfaces::input(&self.font_fields[1], cx).small()),
                cx,
            ))
            .child(
                div().h_flex().justify_end().pt_2().pb_1().child(
                    surfaces::outlined_action("fonts-apply", "check", tr("Apply"), cx).on_click(
                        cx.listener(|this, _, window, cx| {
                            let [text, code] = [0, 1].map(|at| {
                                this.font_fields[at].read(cx).value().trim().to_owned()
                            });
                            let look = crate::theme::Look {
                                text_font: text,
                                code_font: code,
                                ..this.look.clone()
                            };
                            this.change_look(look, window, cx)
                        }),
                    ),
                ),
            )
            .child(explained_setting(
                tr("Send with"),
                "",
                settings::send_key_note(),
                sends,
                cx,
            ))
            .child(setting(
                tr("Workspace folder"),
                if self.workspace_saved {
                    tr("Saved. Wisp uses that folder from its next start.")
                } else {
                    tr("Where the built-in project keeps its files.")
                },
                tipped(
                    "workspace-tip",
                    tr(
                        "Where the built-in project keeps its files. Empty uses the default shown in \
                     the field. Used from Wisp's next start; WISP_GPUI_WORKSPACE overrides it.",
                    ),
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .w(px(200.))
                                .child(surfaces::input(&self.workspace_choice, cx).small()),
                        )
                        .child(
                            surfaces::outlined_action(
                                "workspace-pick",
                                "folder",
                                tr("Choose…"),
                                cx,
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_workspace_choice(window, cx)
                            })),
                        )
                        .child(
                            // The page's one filled button: the folder is the
                            // one choice here that waits for a save.
                            Button::new("workspace-save")
                                .label(tr("Save"))
                                .small()
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_workspace_choice(window, cx)
                                })),
                        ),
                ),
                cx,
            ))
            .child({
                let on = self.separate_storage.unwrap_or(false);
                setting(
                    tr("Each project its own database"),
                    "",
                    tipped(
                        "separate-storage-tip",
                        tr(
                            "New projects keep their conversations in a database inside their own \
                         folder, so the folder can be moved or synced on its own. Projects that \
                         exist stay where they are.",
                        ),
                        switch(
                            "separate-storage",
                            on,
                            tr("Each project its own database"),
                        )
                        .disabled(self.separate_storage.is_none())
                        .on_change(cx.listener(move |this, _, window, cx| {
                            this.set_separate_storage(!on, window, cx)
                        })),
                    ),
                    cx,
                )
            })
            .child(setting(
                tr("Notify me"),
                tr("When a turn ends or a tool waits for approval."),
                tipped(
                    "notify-tip",
                    tr(
                        "When a turn ends or a tool waits for approval while you are in another app, post a system notification. Windows has none without an installed app, so only the sound is available there.",
                    ),
                    switch("notify-banner", self.notify.banner, tr("Notify me")).on_change(
                        cx.listener(|this, _, window, cx| {
                            this.notify.banner = !this.notify.banner;
                            let value = this.notify.banner.to_string();
                            this.save_general(NOTIFY, value, window, cx);
                        }),
                    ),
                ),
                cx,
            ))
            .child(setting(
                tr("Play a sound"),
                "",
                tipped(
                    "notify-sound-tip",
                    tr("Also play a sound then, whether or not a notification is posted."),
                    switch("notify-sound", self.notify.sound, tr("Play a sound")).on_change(
                        cx.listener(|this, _, window, cx| {
                            this.notify.sound = !this.notify.sound;
                            let value = this.notify.sound.to_string();
                            this.save_general(NOTIFY_SOUND, value, window, cx);
                        }),
                    ),
                ),
                cx,
            ))
            .child({
                // Each name plays its sound when chosen, so it can be heard.
                let mut tones = div()
                    .h_flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_1()
                    .max_w(px(380.));
                for (index, tone) in crate::notify::sounds(crate::notify::System::current())
                    .iter()
                    .copied()
                    .enumerate()
                {
                    tones = tones.child(
                        surfaces::choice_chip(("notify-tone", index), self.notify_tone == tone, cx)
                            .child(tone)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.notify_tone = tone;
                                crate::notify::post(
                                    crate::notify::Choice {
                                        banner: false,
                                        sound: true,
                                    },
                                    tone,
                                    crate::notify::Event::Done,
                                    "",
                                );
                                this.save_general(NOTIFY_TONE, tone.into(), window, cx);
                            })),
                    );
                }
                setting(
                    tr("Which sound"),
                    tr("One of the system's sounds; choosing one plays it."),
                    tones,
                    cx,
                )
            })
            .child({
                let (note, available) =
                    settings::update_note(self.update.as_ref(), self.update_at_start);
                let mut actions = div().h_flex().items_center().gap_1().child(
                    surfaces::outlined_action(
                        "update-check",
                        "refresh",
                        tr("Check for updates"),
                        cx,
                    )
                    .tooltip(tr("Nothing is downloaded; the release page opens."))
                    .disabled(self.update == Some(settings::Update::Checking))
                    .on_click(cx.listener(|this, _, window, cx| this.check_for_update(window, cx))),
                );
                if let (true, Some(settings::Update::Found(found))) = (available, &self.update) {
                    let url = found.url.clone();
                    actions = actions.child(
                        surfaces::icon_action(
                            "update-open",
                            "external-link",
                            tr("Open the release page in your browser"),
                            surfaces::IconTone::On,
                            cx,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url))),
                    );
                }
                setting(tr("Updates"), note, actions, cx)
            })
            .child({
                let at_start = self.update_at_start;
                setting(
                    tr("Check at start"),
                    "",
                    switch(
                        "update-at-start",
                        at_start,
                        tr("Also look for a newer version each time Wisp starts"),
                    )
                    .on_change(cx.listener(move |this, _, window, cx| {
                        this.set_update_at_start(!at_start, window, cx)
                    })),
                    cx,
                )
            })
            .child(setting(
                tr("When Wisp starts"),
                "",
                tipped(
                    "launch-tip",
                    tr(
                        "Open on the home page with every project, or on the conversation that \
                 was open when you last used it.",
                    ),
                    starts,
                ),
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
        let mut section = div()
            .v_flex()
            .child(surfaces::settings_heading(tr("Your own"), cx))
            .child(surfaces::settings_note_help(tr("Any other key, given to the code the model runs."),tr(
                "Any other key, given to the code the model runs under the environment variable you name. Adding one with a variable that is already here replaces its value.",
            ), cx).pb_2());
        let rows: Vec<_> = self
            .custom_credentials
            .iter()
            .map(|credential| {
                let id = credential.id.clone();
                let row = surfaces::record_row(
                    SharedString::from(format!("custom-credential-{id}")),
                    credential.name.clone(),
                )
                .icon("lock")
                .suffix(credential.env_var.clone());
                let row = if credential.saved {
                    row.stamp(surfaces::StampKind::Success, tr("Saved"))
                } else {
                    row.stamp(surfaces::StampKind::Waiting, tr("Not set"))
                };
                row.action(
                    surfaces::danger_action(
                        SharedString::from(format!("custom-credential-remove-{id}")),
                        "trash",
                        tr("Remove it and its value from the keychain"),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.remove_custom_credential(id.clone(), window, cx)
                    })),
                )
            })
            .collect();
        if !rows.is_empty() {
            section = section.child(surfaces::record_group(rows, cx));
        }
        let ready = self
            .custom_credential_fields
            .iter()
            .all(|field| !field.read(cx).value().trim().is_empty());
        let mut fields = div().h_flex().items_center().gap_1();
        for field in &self.custom_credential_fields {
            fields = fields.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(surfaces::input(field, cx).small()),
            );
        }
        section.child(
            div().py_3().child(
                fields.child(
                    surfaces::outlined_action("custom-credential-add", "plus", tr("Add"), cx)
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_custom_credential(window, cx)
                        })),
                ),
            ),
        )
    }

    pub(super) fn render_credential_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut page = self.settings_page(cx).child(
            surfaces::settings_note_help(
                tr("Keys for the services the connectors read from."),
                tr(
                    "Keys for the services the connectors read from. They are kept in the \
                     system keychain, never in the project database, and are not shown again \
                     once saved. They are used from the next turn.",
                ),
                cx,
            )
            .pb_2(),
        );
        if self.credentials.is_empty() {
            page = page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        }
        let mut list = div().v_flex().border_t_1().border_color(cx.theme().border);
        for (row, field) in &self.credentials {
            let id = row.id;
            // Save does nothing while the field is empty; an empty value is
            // what Forget is for.
            let mut controls = div()
                .h_flex()
                .items_center()
                .gap_1()
                .child(div().w(px(200.)).child(surfaces::input(field, cx).small()))
                .child(
                    surfaces::outlined_action(
                        SharedString::from(format!("credential-save-{id}")),
                        "check",
                        tr("Save"),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_credential(id, false, window, cx)
                    })),
                );
            if row.saved {
                controls = controls.child(
                    surfaces::danger_action(
                        SharedString::from(format!("credential-forget-{id}")),
                        "trash",
                        tr("Remove it from the keychain"),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.save_credential(id, true, window, cx)
                    })),
                );
            }
            let under = div()
                .v_flex()
                .gap_1()
                .child(surfaces::settings_note(crate::i18n::host(row.note), cx))
                .child(div().h_flex().child(if row.saved {
                    surfaces::stamp(surfaces::StampKind::Success, tr("Saved"), cx)
                } else {
                    surfaces::fact(tr("Not set"), cx)
                }));
            list = list.child(item_row(
                SharedString::from(crate::i18n::host(row.label)),
                Some(under.into_any_element()),
                controls,
                cx,
            ));
        }
        page.child(list).child(self.custom_credential_section(cx))
    }

    pub(super) fn settings_page(&self, cx: &App) -> Div {
        // The column (`surfaces::settings_column`) gives the page its width
        // and inset.
        let mut page = div().v_flex().w_full();
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
        type Step = Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>;
        let compact_step = |up: bool| -> Step {
            let next = settings::step_percent(percent, up, defaults.compact_range);
            (defaults.auto_compact && next != percent).then(|| {
                Box::new(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            auto_compact_percent: next,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                })) as Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>
            })
        };
        let limit = defaults.auto_continue_limit;
        let continue_step = |up: bool| -> Step {
            let next = settings::step_continues(limit, up);
            (defaults.auto_continue && next != limit).then(|| {
                Box::new(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            auto_continue_limit: next,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                })) as Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>
            })
        };
        page.child(explained_setting(
            tr("Steps per turn"),
"",
            crate::i18n::fill(
                tr(
                    "How many times the model may call tools or answer within one turn, from {} \
                 to {}. At the limit it stops and says where it got to.",
                ),
                &[&STEPS.start(), &STEPS.end()],
            ),
            div()
                .h_flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .w(px(90.))
                        .child(surfaces::input(&self.steps_input, cx).small()),
                )
                .child(
                    surfaces::outlined_action("steps-save", "check", tr("Save"), cx)
                        .on_click(cx.listener(|this, _, window, cx| this.save_steps(window, cx))),
                ),
            cx,
        ))
        .child(setting(
            tr("Compact automatically"),
            "",
            tipped(
                "auto-compact-tip",
                tr(
                    "When the context fills up, older turns are summarised so the conversation \
             can go on. Off, a long conversation ends when the model's window is full.",
                ),
                switch(
                    "auto-compact",
                    defaults.auto_compact,
                    tr("Compact automatically"),
                )
                .on_change(cx.listener(move |this, _, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            auto_compact: !defaults.auto_compact,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                })),
            ),
            cx,
        ))
        .child(explained_setting(
            tr("Compact at"),
"",
            crate::i18n::fill(
                tr(
                    "How full the context is when that happens, from {}% to {}%. Lower \
                 compacts sooner and keeps more room for long tool output.",
                ),
                &[&low, &high],
            ),
            count_stepper(
                "compact",
                format!("{percent}%"),
                (tr("Compact sooner"), compact_step(false)),
                (tr("Compact later"), compact_step(true)),
                cx,
            ),
            cx,
        ))
        .child(setting(
            tr("Continue cut-off answers"),
            "",
            tipped(
                "auto-continue-tip",
                tr(
                    "When an answer stops at the model's output limit, the turn carries on by \
                 itself, at most this many times. Off, a cut answer ends the turn.",
                ),
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(count_stepper(
                        "continue",
                        limit.to_string(),
                        (tr("Fewer times"), continue_step(false)),
                        (tr("More times"), continue_step(true)),
                        cx,
                    ))
                    .child(
                        switch(
                            "auto-continue",
                            defaults.auto_continue,
                            tr("Continue cut-off answers"),
                        )
                        .on_change(cx.listener(
                            move |this, _, window, cx| {
                                this.save_turn_defaults(
                                    TurnDefaults {
                                        auto_continue: !defaults.auto_continue,
                                        ..defaults
                                    },
                                    window,
                                    cx,
                                )
                            },
                        )),
                    ),
            ),
            cx,
        ))
        .child(setting(
            tr("Suggest what to ask next"),
            "",
            tipped(
                "follow-ups-tip",
                tr(
                    "After an answer the model is asked once more, briefly, for up to three \
             questions you could ask next. They show above the composer. Off, that extra \
             request is not made.",
                ),
                switch(
                    "follow-ups",
                    defaults.follow_ups,
                    tr("Suggest what to ask next"),
                )
                .on_change(cx.listener(move |this, _, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            follow_ups: !defaults.follow_ups,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                })),
            ),
            cx,
        ))
        .child(setting(
            tr("Review turns"),
            "",
            tipped(
                "auto-review-tip",
                tr(
                    "When a turn that used a tool, or said a good deal, ends, a reviewer checks its \
             claims against the tool output and the result shows above the composer. This \
             is what conversations start with; each can be switched in its composer.",
                ),
                switch(
                    "auto-review-default",
                    defaults.auto_review,
                    tr("Review turns"),
                )
                .on_change(cx.listener(move |this, _, window, cx| {
                    this.save_turn_defaults(
                        TurnDefaults {
                            auto_review: !defaults.auto_review,
                            ..defaults
                        },
                        window,
                        cx,
                    )
                })),
            ),
            cx,
        ))
        .children(
            self.failure_analysis
                .map(|failures| self.failure_analysis_setting(failures, cx)),
        )
        .child(surfaces::settings_heading(tr("Models"), cx))
        .child(setting(
            tr("Reviewer model"),
            "",
            tipped(
                "review-model-tip",
                tr(
                    "The model that reviews, for automatic reviews and those you ask for. A second \
             model catches more than a model checking itself.",
                ),
                self.review_model_choices(cx),
            ),
            cx,
        ))
        .child(setting(
            tr("Sub-agent model"),
            "",
            tipped(
                "subagent-model-tip",
                tr(
                    "The model that helpers the main model sends off to explore work on. A \
                 cheaper, faster model here keeps long searches off the conversation's model.",
                ),
                self.subagent_model_choices(cx),
            ),
            cx,
        ))
        .child(setting(
            tr("Picture model"),
            "",
            tipped(
                "picture-model-tip",
                tr(
                    "When the conversation's model cannot see, this one looks at the pictures you \
                 attach and tells it what they show. Only models that can see are listed.",
                ),
                self.picture_model_choices(cx),
            ),
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
        let step =
            |rate: bool, up: bool| -> Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>> {
                let next = settings::step_failures(failures, rate, up);
                (failures.enabled && next != failures).then(|| {
                    Box::new(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.save_failure_analysis(next, window, cx)
                    })) as Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>
                })
            };
        let rate = count_stepper(
            "failure-rate",
            format!("{}%", failures.failure_rate_threshold),
            (tr("A smaller share"), step(true, false)),
            (tr("A larger share"), step(true, true)),
            cx,
        );
        let count = count_stepper(
            "failure-count",
            failures.minimum_failures.to_string(),
            (tr("Fewer failures"), step(false, false)),
            (tr("More failures"), step(false, true)),
            cx,
        );
        let controls = div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(rate)
            .child(count)
            .child(
                switch(
                    "failure-analysis",
                    failures.enabled,
                    tr("Draft a memory when tools fail"),
                )
                .on_change(cx.listener(move |this, _, window, cx| {
                    let next = wisp_server::desktop::AutoFailureAnalysisSettings {
                        enabled: !failures.enabled,
                        ..failures
                    };
                    this.save_failure_analysis(next, window, cx)
                })),
            );
        setting(
            tr("Draft a memory when tools fail"),
            "",
            tipped(
                "failure-analysis-tip",
                tr(
                    "When this share of a turn's tool calls failed, and at least this many, the \
             model drafts what to remember about why, for you to confirm. A turn that asks \
             Wisp to remember something has a draft either way.",
                ),
                controls,
            ),
            cx,
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
        let mut choices = model_chips().child(
            surfaces::choice_chip("subagent-model-own", own, cx)
                .child(tr("The conversation's"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.save_subagent_model(String::new(), window, cx)
                })),
        );
        for profile in profiles {
            let id = profile.id.clone();
            choices = choices.child(
                surfaces::choice_chip(
                    SharedString::from(format!("subagent-model-{}", profile.id)),
                    profile.id == current,
                    cx,
                )
                .child(profile.name.clone())
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
        let none = profiles.is_empty();
        let mut choices = model_chips().child(
            surfaces::choice_chip("picture-model-first", first, cx)
                .child(if none {
                    tr("None can see")
                } else {
                    tr("The first that can see")
                })
                .when(none, |chip| chip.opacity(0.5))
                .when(!none, |chip| {
                    chip.on_click(cx.listener(|this, _, window, cx| {
                        this.save_picture_model(String::new(), window, cx)
                    }))
                }),
        );
        for profile in profiles {
            let id = profile.id.clone();
            choices = choices.child(
                surfaces::choice_chip(
                    SharedString::from(format!("picture-model-{}", profile.id)),
                    profile.id == current,
                    cx,
                )
                .child(profile.name.clone())
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
        let mut choices = model_chips().child(
            surfaces::choice_chip("review-model-own", own, cx)
                .child(tr("The conversation's"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.save_review_model(String::new(), window, cx)
                })),
        );
        for profile in profiles {
            let id = profile.id.clone();
            choices = choices.child(
                surfaces::choice_chip(
                    SharedString::from(format!("review-model-{}", profile.id)),
                    profile.id == current,
                    cx,
                )
                .child(profile.name.clone())
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
        let mode =
            |id: &'static str, icon_name: &'static str, label: &'static str, full_access: bool| {
                surfaces::segment(
                    id,
                    icon_name,
                    label,
                    defaults.full_access == full_access,
                    cx,
                )
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
                .into_any_element()
            };
        let mut page = page.child(setting(
            tr("New conversations start in"),
            "",
            tipped(
                "default-access-tip",
                tr(
                    "Ask first: tools that change files or run commands wait for your answer. \
             Full access: they run at once. A dangerous shell command asks either way. \
             Conversations you already have keep their own choice.",
                ),
                surfaces::segmented(
                    [
                        mode("default-ask", "hand", tr("Ask first"), false),
                        mode("default-full", "shield", tr("Full access"), true),
                    ],
                    cx,
                ),
            ),
            cx,
        ));
        page = page.child(self.approval_settings(cx));
        page = page
            .child(surfaces::settings_heading(tr("Always allowed"), cx))
            .child(
                surfaces::settings_note_help(
                    tr("Tools you always allow, by project."),
                    tr(
                        "Tools you answered \"Always in this project\" for. They run without \
                     asking in every conversation of that project until you take them back.",
                    ),
                    cx,
                )
                .pb_2(),
            );
        let grant_row =
            |id: SharedString,
             tool: &String,
             revoke: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>| {
                surfaces::record_row(id.clone(), tool.clone())
                    .icon("wrench")
                    .mono()
                    .action(
                        surfaces::icon_action(
                            SharedString::from(format!("{id}-revoke")),
                            "close",
                            tr("Ask again before this tool runs"),
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .on_click(move |event, window, cx| revoke(event, window, cx)),
                    )
            };
        if !self.global_grants.is_empty() {
            let rows: Vec<_> = self
                .global_grants
                .iter()
                .map(|tool| {
                    let name = tool.clone();
                    grant_row(
                        SharedString::from(format!("revoke-everywhere-{tool}")),
                        tool,
                        Box::new(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.revoke_global_grant(name.clone(), window, cx)
                        })),
                    )
                })
                .collect();
            page = page
                .child(surfaces::settings_heading(tr("Every project"), cx))
                .child(surfaces::record_group(rows, cx));
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
            let rows: Vec<_> = grants
                .tools
                .iter()
                .map(|tool| {
                    let (project, name) = (grants.project_id.clone(), tool.clone());
                    grant_row(
                        SharedString::from(format!("revoke-{}-{tool}", grants.project_id)),
                        tool,
                        Box::new(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.revoke_grant(project.clone(), name.clone(), window, cx)
                        })),
                    )
                })
                .collect();
            page = page
                .child(surfaces::settings_heading(grants.project_name.clone(), cx))
                .child(surfaces::record_group(rows, cx));
        }
        page
    }
}
