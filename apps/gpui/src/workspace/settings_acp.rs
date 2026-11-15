//! The ACP agents page of Settings: programs on this computer that speak
//! the Agent Client Protocol. The form and the remove question are layers
//! inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::{AcpAgent, AcpAgentRow};

use super::{Workspace, chip, shell::glyph};
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

    fn acp_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(620.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(label(tr("Name")))
            .child(Input::new(&self.acp_fields[0]).small())
            .child(label(tr(
                "The program that starts it: a name found on PATH or a full path",
            )))
            .child(Input::new(&self.acp_fields[1]).small())
            .child(label(tr(
                "Its arguments, on one line; quote one that has a space",
            )))
            .child(Input::new(&self.acp_fields[2]).small())
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_1()
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_acp_forms();
                                cx.notify();
                            })),
                    ),
            )
    }

    fn acp_row(&self, row: &AcpAgentRow, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let agent = &row.agent;
        let id = agent.id.clone();
        let asking = self.layers.contains(Layer::ConfirmRemoveAcpAgent)
            && self.acp_editing.as_deref() == Some(id.as_str());
        let mut about = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(agent.label.clone()),
                    )
                    .when(!row.found, |line| {
                        line.child(chip(tr("Program not found"), Tone::Attention, cx))
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .font_family("Menlo")
                    .truncate()
                    .child(acp::command_line(&agent.command, &agent.args)),
            );
        if self.acp_testing.as_deref() == Some(id.as_str()) {
            about = about.child(div().text_xs().text_color(muted).child(tr("Starting…")));
        } else if let Some(report) = self.acp_reports.get(&id) {
            about = about.child(match report {
                Ok(report) => chip(acp::report_line(report), Tone::Success, cx),
                Err(error) => chip(crate::i18n::host(error), Tone::Danger, cx),
            });
        }
        let mut actions = div().h_flex().gap_1().flex_shrink_0();
        if asking {
            let removed = id.clone();
            actions = actions
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
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_acp_forms();
                            cx.notify();
                        })),
                );
        } else {
            let (tested, editing, removing) = (id.clone(), agent.clone(), id.clone());
            actions = actions
                .child(
                    Button::new(SharedString::from(format!("acp-test-{id}")))
                        .label(tr("Test"))
                        .small()
                        .ghost()
                        .disabled(self.acp_testing.is_some())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.test_acp_agent(tested.clone(), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("acp-edit-{id}")))
                        .label(tr("Change"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_acp_agent(Some(&editing), window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("acp-remove-{id}")))
                        .label(tr("Remove"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.layers.close(Layer::AcpAgentForm);
                            this.acp_editing = Some(removing.clone());
                            this.layers.open(Layer::ConfirmRemoveAcpAgent);
                            cx.notify();
                        })),
                );
        }
        let mut shown = div()
            .h_flex()
            .items_start()
            .gap_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(about)
            .child(actions);
        if asking {
            shown = div().v_flex().child(shown).child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(tr(
                        "Remove this agent from the list? The program itself stays installed.",
                    )),
            );
        }
        shown
    }

    pub(super) fn render_acp_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut page = self.settings_page(cx).child(
            div()
                .pb_2()
                .w(px(620.))
                .text_xs()
                .text_color(muted)
                .child(tr(
                    "Agents on this computer that speak the Agent Client Protocol, such as a \
                 coding agent with an ACP adapter. Test starts one, reads its name and how it \
                 signs in, and stops it. The list is shared with the WebView desktop.",
                )),
        );
        let Some(agents) = &self.acp_agents else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let form_open = self.layers.contains(Layer::AcpAgentForm);
        let editing = self.acp_editing.as_deref();
        if agents.is_empty() && !form_open {
            page = page.child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("None yet.")),
            );
        }
        for row in agents {
            page = page.child(self.acp_row(row, cx));
            if form_open && editing == Some(row.agent.id.as_str()) {
                page = page.child(div().py_2().child(self.acp_form(cx)));
            }
        }
        if form_open && editing == Some("") {
            page.child(div().py_2().child(self.acp_form(cx)))
        } else {
            page.child(
                div().pt_3().h_flex().child(
                    Button::new("acp-add")
                        .icon(glyph("plus"))
                        .label(tr("Add an agent"))
                        .small()
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.edit_acp_agent(None, window, cx)
                            }),
                        ),
                ),
            )
        }
    }
}
