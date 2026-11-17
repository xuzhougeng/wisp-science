//! The ACP agents page of Settings: programs on this computer that speak
//! the Agent Client Protocol. The form and the remove question are layers
//! inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{AcpAgent, AcpAgentRow, AcpSignInKind, AcpSignInOutcome};

use super::{Workspace, chip, settings_sections::setting, surfaces};
use crate::{acp, backend::receive, layers::Layer, present::Tone};

impl Workspace {
    pub(super) fn load_acp_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.acp_agents();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(agents) => this.acp_agents = Some(agents),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the form on a new agent, or on `agent` to change it.
    fn edit_acp_agent(
        &mut self,
        agent: Option<&AcpAgent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let blank = AcpAgent::default();
        let shown = agent.unwrap_or(&blank);
        self.acp_editing = Some(shown.id.clone());
        self.layers.close(Layer::ConfirmRemoveAcpAgent);
        self.layers.open(Layer::AcpAgentForm);
        self.settings_error = None;
        let values = [
            shown.label.clone(),
            shown.command.clone(),
            acp::join_args(&shown.args),
        ];
        for (field, value) in self.acp_fields.iter().zip(values) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.acp_fields[0].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close_acp_forms(&mut self) {
        self.layers.close(Layer::AcpAgentForm);
        self.layers.close(Layer::ConfirmRemoveAcpAgent);
        self.acp_editing = None;
    }

    fn save_acp_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.acp_editing.clone() else {
            return;
        };
        let [label, command, args] =
            [0, 1, 2].map(|at| self.acp_fields[at].read(cx).value().to_string());
        let reply = self.backend.save_acp_agent(AcpAgent {
            id,
            label,
            command,
            args: acp::split_args(&args),
        });
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(saved) => {
                        this.acp_reports.remove(&saved);
                        this.close_acp_forms();
                        this.load_acp_agents(window, cx);
                    }
                    // The form stays open with what was typed.
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_acp_agent(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.remove_acp_agent(id.clone());
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.close_acp_forms();
                match result {
                    Ok(()) => {
                        this.acp_reports.remove(&id);
                        this.load_acp_agents(window, cx);
                    }
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Starts the agent and shows how it introduced itself, or why it
    /// could not start.
    fn test_acp_agent(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.acp_testing.is_some() {
            return;
        }
        self.acp_testing = Some(id.clone());
        self.acp_reports.remove(&id);
        let reply = self.backend.test_acp_agent(id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.acp_testing = None;
                this.acp_reports.insert(id, result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Signs in to an agent one of the ways it offers. One that signs in
    /// in a terminal gets the system's terminal, in the open project's
    /// folder or the home folder.
    fn sign_in_acp_agent(
        &mut self,
        id: String,
        method: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.acp_testing.is_some() {
            return;
        }
        self.acp_testing = Some(id.clone());
        self.acp_signed.remove(&id);
        let folder = self
            .state
            .project
            .as_ref()
            .and_then(|project| self.projects.iter().find(|row| &row.id == project))
            .map(|project| std::path::PathBuf::from(&project.workspace_dir))
            .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
            .unwrap_or_else(std::env::temp_dir);
        let reply = self.backend.sign_in_acp_agent(id.clone(), method);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.acp_testing = None;
                let said = match result {
                    Ok(AcpSignInOutcome::Done) => Ok(tr("Signed in.").to_owned()),
                    Ok(AcpSignInOutcome::Terminal { program, args, env }) => {
                        crate::terminal::run(&folder, &program, &args, &env).map(|()| {
                            tr("Finish signing in in the terminal that opened, then test again.")
                                .to_owned()
                        })
                    }
                    Err(error) => Err(error),
                };
                this.acp_signed.insert(id, said);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The form of a new or changed agent, under the list in the same
    /// column: a setting row for each field, then Save and Cancel.
    fn acp_form(&self, cx: &mut Context<Self>) -> Div {
        let field = |at: usize| {
            div()
                .w(px(300.))
                .child(surfaces::input(&self.acp_fields[at], cx))
        };
        let adding = self.acp_editing.as_deref() == Some("");
        div()
            .v_flex()
            .w_full()
            .child(surfaces::settings_heading(
                if adding {
                    tr("Add an agent")
                } else {
                    tr("Change")
                },
                cx,
            ))
            .child(setting(tr("Name"), "", field(0), cx))
            .child(setting(
                tr("Program"),
                tr("The program that starts it: a name found on PATH or a full path"),
                field(1),
                cx,
            ))
            .child(setting(
                tr("Arguments"),
                tr("Its arguments, on one line; quote one that has a space"),
                field(2),
                cx,
            ))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_3()
                    .child(
                        Button::new("acp-save")
                            .label(tr("Save"))
                            .small()
                            .primary()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_acp_agent(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("acp-cancel")
                            .label(tr("Cancel"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_acp_forms();
                                cx.notify();
                            })),
                    ),
            )
    }

    /// One agent as a record row: its name, its command line after it, a
    /// stamp when its program is missing; Test outlined, Change and Remove
    /// as icon buttons. What a test found, the ways to sign in and the
    /// remove question open under it.
    fn acp_row(&self, row: &AcpAgentRow, cx: &mut Context<Self>) -> surfaces::RecordRow {
        let muted = cx.theme().muted_foreground;
        let agent = &row.agent;
        let id = agent.id.clone();
        let asking = self.layers.contains(Layer::ConfirmRemoveAcpAgent)
            && self.acp_editing.as_deref() == Some(id.as_str());
        let mut under = div().v_flex().gap_1();
        let mut opened = false;
        if self.acp_testing.as_deref() == Some(id.as_str()) {
            opened = true;
            under = under.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Starting the agent…")),
            );
        } else if let Some(report) = self.acp_reports.get(&id) {
            opened = true;
            under = under.child(match report {
                Ok(report) => chip(acp::report_line(report), Tone::Success, cx),
                Err(error) => chip(crate::i18n::host(error), Tone::Danger, cx),
            });
            // The ways it offers to be signed in to, each an icon and a
            // word; one that reads a key from the environment is only
            // named.
            if let Ok(report) = report {
                let mut ways = div().h_flex().flex_wrap().gap_1();
                for way in &report.sign_in {
                    if way.kind == AcpSignInKind::Environment {
                        continue;
                    }
                    let (agent, method) = (id.clone(), way.id.clone());
                    ways = ways.child(
                        surfaces::labelled_action(
                            SharedString::from(format!("acp-sign-in-{id}-{}", way.id)),
                            match way.kind {
                                AcpSignInKind::Terminal => "terminal",
                                _ => "user",
                            },
                            crate::i18n::fill(tr("Sign in: {}"), &[&way.name]),
                            surfaces::IconSize::Inline,
                            surfaces::IconTone::Faded,
                            cx,
                        )
                        .disabled(self.acp_testing.is_some())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.sign_in_acp_agent(agent.clone(), method.clone(), window, cx)
                            },
                        )),
                    );
                }
                under = under.child(ways);
            }
        }
        if let Some(said) = self.acp_signed.get(&id) {
            opened = true;
            under = under.child(match said {
                Ok(said) => chip(said.clone(), Tone::Success, cx),
                Err(error) => chip(crate::i18n::host(error), Tone::Danger, cx),
            });
        }
        if asking {
            opened = true;
            let removed = id.clone();
            under = under.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(tr(
                                "Remove this agent from the list? The program itself stays installed.",
                            )),
                    )
                    .child(
                        Button::new(SharedString::from(format!("acp-remove-yes-{id}")))
                            .label(tr("Remove"))
                            .small()
                            .danger()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.remove_acp_agent(removed.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("acp-remove-no-{id}")))
                            .label(tr("Keep"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_acp_forms();
                                cx.notify();
                            })),
                    ),
            );
        }
        let (tested, editing, removing) = (id.clone(), agent.clone(), id.clone());
        let mut shown =
            surfaces::record_row(SharedString::from(format!("acp-{id}")), agent.label.clone())
                .icon("swarm")
                .suffix(acp::command_line(&agent.command, &agent.args))
                .action(
                    surfaces::outlined_action(
                        SharedString::from(format!("acp-test-{id}")),
                        "refresh",
                        tr("Test"),
                        cx,
                    )
                    .disabled(self.acp_testing.is_some())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.test_acp_agent(tested.clone(), window, cx)
                    })),
                )
                .action(
                    surfaces::icon_action(
                        SharedString::from(format!("acp-edit-{id}")),
                        "edit",
                        tr("Change"),
                        surfaces::IconTone::Faded,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_acp_agent(Some(&editing), window, cx)
                    })),
                )
                .action(
                    surfaces::danger_action(
                        SharedString::from(format!("acp-remove-{id}")),
                        "trash",
                        tr("Remove"),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.layers.close(Layer::AcpAgentForm);
                        this.acp_editing = Some(removing.clone());
                        this.layers.open(Layer::ConfirmRemoveAcpAgent);
                        cx.notify();
                    })),
                );
        if !row.found {
            shown = shown.stamp(surfaces::StampKind::Waiting, tr("Program not found"));
        }
        shown.below(opened.then(|| under.into_any_element()))
    }

    pub(super) fn render_acp_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut page = self.settings_page(cx).child(
            surfaces::settings_note_help(
                tr("Programs on this computer that speak the Agent Client Protocol."),
                tr(
                    "Agents on this computer that speak the Agent Client Protocol, such as a \
                 coding agent with an ACP adapter. Test starts one, reads its name and how it \
                 signs in, and stops it. The list is shared with the WebView desktop.",
                ),
                cx,
            )
            .pb_3(),
        );
        let Some(agents) = &self.acp_agents else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let form_open = self.layers.contains(Layer::AcpAgentForm);
        if agents.is_empty() && !form_open {
            page = page.child(surfaces::settings_note(tr("None yet."), cx).pb_2());
        }
        if !agents.is_empty() {
            let editing = self.acp_editing.as_deref();
            let hover = cx.theme().list_hover;
            let paper = cx.theme().popover;
            let rows: Vec<_> = agents
                .iter()
                .map(|row| {
                    let chosen = form_open && editing == Some(row.agent.id.as_str());
                    self.acp_row(row, cx)
                        .when(chosen, |shown| shown.bg(paper))
                        .when(!chosen, |shown| shown.hover(move |style| style.bg(hover)))
                })
                .collect();
            page = page.child(surfaces::record_group(rows, cx));
        }
        if form_open {
            page.child(self.acp_form(cx))
        } else {
            page.child(div().pt_3().h_flex().child(
                surfaces::outlined_action("acp-add", "plus", tr("Add an agent"), cx).on_click(
                    cx.listener(|this, _, window, cx| this.edit_acp_agent(None, window, cx)),
                ),
            ))
        }
    }
}
