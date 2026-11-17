//! Wisp Lab on the Chat apps page of Settings: joining a research group
//! hosted by a wisp-relay, the members (approved and removed by the
//! leader), invite codes, and the project whose conversations answer
//! other members' agents.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::LabStatus;

use super::{
    Workspace,
    settings_sections::{explained_setting, setting},
    shell::glyph,
    surfaces::{self, IconTone},
    tone_color,
};
use crate::{
    backend::{Reply, receive},
    channels,
    layers::Layer,
    present::Tone,
};

/// The lab form's fields, in `lab_fields`.
const RELAY: usize = 0;
const TOKEN: usize = 1;
const NAME: usize = 2;
const LAB_NAME: usize = 3;
const INVITE: usize = 4;

impl Workspace {
    pub(super) fn load_lab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.lab_status();
        self.receive_lab(reply, window, cx);
    }

    fn receive_lab(
        &mut self,
        reply: Reply<LabStatus>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lab_busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                this.lab_busy = false;
                match result {
                    Ok(status) => this.lab = Some(status),
                    Err(error) => this.lab_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs a change, then reads the lab again.
    fn lab_change(
        &mut self,
        reply: Reply<()>,
        said: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lab_note = None;
        self.lab_busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.lab_busy = false;
                match result {
                    Ok(()) => {
                        this.lab_note = said.map(|said| Ok(tr(said).to_owned()));
                        this.load_lab(window, cx);
                    }
                    Err(error) => this.lab_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn join_lab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lab_note = None;
        let [relay, token, name, lab_name, invite, _] = self
            .lab_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        let token = token.trim().to_owned();
        let reply = self.backend.join_lab(
            relay,
            (!token.is_empty()).then_some(token),
            name,
            lab_name,
            invite,
        );
        for field in [TOKEN, INVITE] {
            self.lab_fields[field].update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.receive_lab(reply, window, cx);
    }

    fn create_lab_invite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lab_note = None;
        let reply = self.backend.create_lab_invite();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(code) => this.lab_invite = Some(code),
                    Err(error) => this.lab_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_lab_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let token = self.lab_fields[5].read(cx).value().trim().to_owned();
        if token.is_empty() {
            return;
        }
        self.lab_fields[5].update(cx, |input, cx| input.set_value("", window, cx));
        let reply = self.backend.set_lab_relay_token(token);
        self.lab_change(reply, Some("The new relay token is kept."), window, cx);
    }

    fn leave_lab(&mut self, forget: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmLeaveLab);
        self.lab_invite = None;
        let reply = self.backend.leave_lab(forget);
        self.lab_change(reply, None, window, cx);
    }

    fn remove_lab_member(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmRemoveLabMember);
        let Some(member) = self.lab_removing.take() else {
            return;
        };
        let reply = self.backend.remove_lab_member(member);
        self.lab_change(reply, None, window, cx);
    }

    /// The form that founds a lab or joins one with an invite code.
    fn lab_join_form(&self, cx: &mut Context<Self>) -> Div {
        let [relay, _, name, _, invite, _] = self
            .lab_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        let founding = invite.trim().is_empty();
        let ready = !name.trim().is_empty() && (!founding || !relay.trim().is_empty());
        let field = |index: usize, width: f32| {
            div()
                .w(px(width))
                .child(surfaces::input(&self.lab_fields[index], cx).small())
        };
        div()
            .v_flex()
            .child(explained_setting(
                tr("Invite code"),
                "",
                tr("From the leader; leave it empty to found the relay's lab and lead it."),
                field(INVITE, 300.),
                cx,
            ))
            .child(explained_setting(
                tr("Relay server URL"),
                "",
                tr("The wisp-relay that hosts the lab; an invite code already names it."),
                field(RELAY, 300.),
                cx,
            ))
            .child(explained_setting(
                tr("Relay access token"),
                "",
                tr("Given by the relay's operator or the leader; kept in the system keychain."),
                field(TOKEN, 220.),
                cx,
            ))
            .child(setting(
                tr("Your name"),
                tr("What other members see."),
                field(NAME, 220.),
                cx,
            ))
            .when(founding, |form| {
                form.child(setting(
                    tr("Lab name"),
                    tr("Shown to everyone who joins."),
                    field(LAB_NAME, 220.),
                    cx,
                ))
            })
            .child(
                div().h_flex().pt_2().child(
                    Button::new("lab-join")
                        .icon(glyph(if founding { "plus" } else { "arrow-right" }))
                        .label(if founding {
                            tr("Found the lab")
                        } else {
                            tr("Ask to join")
                        })
                        .small()
                        .primary()
                        .disabled(!ready || self.lab_busy)
                        .on_click(cx.listener(|this, _, window, cx| this.join_lab(window, cx))),
                ),
            )
    }

    /// The members as record rows: who, a waiting stamp while they ask to
    /// join, and for the leader approve and remove at the right end, the
    /// question to remove under the row it is about.
    fn lab_members(&self, status: &LabStatus, cx: &mut Context<Self>) -> Div {
        let mut rows = Vec::new();
        for member in &status.members {
            let mut row = surfaces::record_row(
                SharedString::from(format!("lab-member-{}", member.id)),
                channels::member_label(member, &status.member_id),
            )
            .icon("user");
            if member.pending {
                row = row.stamp(surfaces::StampKind::Waiting, tr("Waiting for approval"));
            }
            if status.leader && member.id != status.member_id {
                if member.pending {
                    let id = member.id.clone();
                    row = row.action(
                        surfaces::icon_action(
                            SharedString::from(format!("lab-approve-{}", member.id)),
                            "check",
                            tr("Approve"),
                            IconTone::Faded,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                let reply = this.backend.approve_lab_member(id.clone());
                                this.lab_change(reply, None, window, cx);
                            },
                        )),
                    );
                }
                let id = member.id.clone();
                row = row.action(
                    surfaces::danger_action(
                        SharedString::from(format!("lab-remove-{}", member.id)),
                        "trash",
                        if member.pending {
                            tr("Refuse")
                        } else {
                            tr("Remove")
                        },
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.lab_removing = Some(id.clone());
                        this.layers.open(Layer::ConfirmRemoveLabMember);
                        cx.notify();
                    })),
                );
            }
            let asking = self.layers.contains(Layer::ConfirmRemoveLabMember)
                && self.lab_removing.as_deref() == Some(member.id.as_str());
            row = row.below(asking.then(|| {
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(cx.theme().danger)
                            .child(tr("Remove them from the lab?")),
                    )
                    .child(
                        Button::new("lab-remove-yes")
                            .label(tr("Remove"))
                            .xsmall()
                            .danger()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.remove_lab_member(window, cx)
                            })),
                    )
                    .child(
                        Button::new("lab-remove-no")
                            .label(tr("Keep"))
                            .xsmall()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.close(Layer::ConfirmRemoveLabMember);
                                this.lab_removing = None;
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            }));
            rows.push(row);
        }
        if status.members.is_empty() {
            return surfaces::settings_note(tr("No members yet."), cx);
        }
        surfaces::record_group(rows, cx)
    }

    /// The project whose conversations answer lab mail, one button each.
    fn lab_inbox(&self, status: &LabStatus, cx: &mut Context<Self>) -> Div {
        let mut choices = div().flex().flex_wrap().justify_end().gap_1().child(
            surfaces::choice_chip("lab-inbox-none", status.inbox_project_id.is_empty(), cx)
                .child(tr("None"))
                .on_click(cx.listener(|this, _, window, cx| {
                    let reply = this.backend.set_lab_inbox_project(String::new());
                    this.lab_change(reply, None, window, cx);
                })),
        );
        for project in &self.projects {
            let id = project.id.clone();
            choices = choices.child(
                surfaces::choice_chip(
                    SharedString::from(format!("lab-inbox-{}", project.id)),
                    status.inbox_project_id == project.id,
                    cx,
                )
                .child(project.name.clone())
                .on_click(cx.listener(move |this, _, window, cx| {
                    let reply = this.backend.set_lab_inbox_project(id.clone());
                    this.lab_change(reply, None, window, cx);
                })),
            );
        }
        choices
    }

    /// Downloads the lab's knowledge base into a folder the user picks.
    fn download_knowledge_base(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lab_note = None;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Put the knowledge base in this folder").into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let paths = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                match paths.and_then(|mut paths| paths.pop()) {
                    Some(parent) => receive(backend.join_lab_knowledge_base(parent))
                        .await
                        .map(Some),
                    None => Ok(None),
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Some(_)) => {
                        this.lab_note = Some(Ok(tr(
                            "The knowledge base is on this computer; agents here read it.",
                        )
                        .to_owned()));
                        this.load_projects(window, cx);
                        this.load_lab(window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.lab_note = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The lab's knowledge base: the leader publishes a synced project as
    /// it; a member downloads it.
    fn lab_knowledge_base(&self, status: &LabStatus, cx: &mut Context<Self>) -> Div {
        let published = status.knowledge_base.as_ref();
        if status.leader {
            let mut choices = div().flex().flex_wrap().justify_end().gap_1();
            for project in &self.projects {
                let id = project.id.clone();
                choices = choices.child(
                    surfaces::choice_chip(
                        SharedString::from(format!("lab-kb-{}", project.id)),
                        published.is_some_and(|kb| kb.project_id == project.id),
                        cx,
                    )
                    .child(project.name.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let reply = this.backend.publish_lab_knowledge_base(id.clone());
                        this.lab_change(
                            reply,
                            Some("The knowledge base is published to the lab."),
                            window,
                            cx,
                        );
                    })),
                );
            }
            return div().child(explained_setting(
                tr("Knowledge base"),
"",
                tr("A project synchronized through the lab's relay that members download and their agents read. Sync it once from its settings first."),
                choices,
                cx,
            ));
        }
        let (note, action) = match published {
            None => (tr("The leader has not published one yet.").to_owned(), None),
            Some(kb) if kb.local => (
                crate::i18n::fill(
                    tr("{}, on this computer; agents here read it."),
                    &[&kb.project_name],
                ),
                None,
            ),
            Some(_) => (
                tr("The leader published one; download it so agents here can read it.").to_owned(),
                Some(
                    surfaces::outlined_action(
                        "lab-kb-download",
                        "download",
                        tr("Download it…"),
                        cx,
                    )
                    .on_click(
                        cx.listener(|this, _, window, cx| this.download_knowledge_base(window, cx)),
                    ),
                ),
            ),
        };
        div().child(setting(
            tr("Knowledge base"),
            "",
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(surfaces::settings_note(note, cx))
                .when_some(action, |row, action| row.child(action)),
            cx,
        ))
    }

    pub(super) fn lab_card(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut card = div().v_flex().gap_1().child(surfaces::settings_note_help(
            tr("A research group whose agents can ask each other questions."),
            tr(
                "A research group whose members' agents can ask each other questions. The \
                 wisp-relay that hosts it knows who belongs; the lab key stays on the members' \
                 computers and seals every mail. A member's agent can list the lab and mail a \
                 member; the answer comes back into the asking conversation.",
            ),
            cx,
        ));
        let Some(status) = &self.lab else {
            return card.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let (line, tone) = channels::lab_line(status);
        card = card.child(div().text_xs().text_color(tone_color(tone, cx)).child(line));
        match status.state.as_str() {
            "none" => card = card.child(self.lab_join_form(cx)),
            "active" | "pending" => {
                if status.state == "active" {
                    card = card
                        .child(surfaces::settings_heading(tr("Members"), cx))
                        .child(self.lab_members(status, cx));
                    card = card.child(explained_setting(
                        tr("Answer lab mail in"),
"",
                        tr("Other members' agents are answered in a conversation per sender in this project. With none, they are told this computer does not answer."),
                        self.lab_inbox(status, cx),
                        cx,
                    ));
                }
                if status.leader {
                    let mut invite = div().h_flex().items_center().gap_1().child(
                        surfaces::outlined_action(
                            "lab-invite",
                            "plus",
                            tr("Make an invite code"),
                            cx,
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.create_lab_invite(window, cx)),
                        ),
                    );
                    if let Some(code) = &self.lab_invite {
                        let code = code.clone();
                        invite = invite
                            .child(surfaces::read_only(code.clone(), cx).max_w(px(200.)))
                            .child(
                                surfaces::icon_action(
                                    "lab-invite-copy",
                                    "copy",
                                    tr("Copy"),
                                    IconTone::Faded,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            code.clone(),
                                        ))
                                    },
                                )),
                            );
                    }
                    card = card.child(explained_setting(
                        tr("Invite code"),
"",
                        tr("Each code lets one person ask to join; it carries the lab key, so give it to them yourself, and the relay token separately."),
                        invite,
                        cx,
                    ));
                }
                card = card.child(setting(
                    tr("Relay access token"),
                    tr("Type a new one when the relay's operator changed it."),
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(220.))
                                .child(surfaces::input(&self.lab_fields[5], cx).small()),
                        )
                        .child(
                            surfaces::outlined_action("lab-token-save", "save", tr("Save"), cx)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_lab_token(window, cx)
                                })),
                        ),
                    cx,
                ));
                let asking = self.layers.contains(Layer::ConfirmLeaveLab);
                card = card.child(if asking {
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .pt_2()
                        .child(div().flex_1().min_w_0().text_sm().text_color(cx.theme().danger).child(tr(
                            "Leave the lab? Its keys are dropped from this computer and joining again needs a new invite.",
                        )))
                        .child(
                            Button::new("lab-leave-yes")
                                .label(tr("Leave"))
                                .small()
                                .danger()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.leave_lab(false, window, cx)
                                })),
                        )
                        .child(
                            Button::new("lab-leave-no")
                                .label(tr("Keep"))
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.layers.close(Layer::ConfirmLeaveLab);
                                    cx.notify();
                                })),
                        )
                } else {
                    div().h_flex().pt_2().child(
                        surfaces::danger_action("lab-leave", "ban", tr("Leave the lab"), cx)
                            .disabled(status.leader)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.layers.open(Layer::ConfirmLeaveLab);
                                cx.notify();
                            })),
                    )
                });
            }
            // Removed, or the relay cannot be reached: this computer can
            // only forget the lab, or look again.
            _ => {
                card = card.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .pt_2()
                        .child(
                            surfaces::outlined_action("lab-retry", "refresh", tr("Look again"), cx)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.load_lab(window, cx)),
                                ),
                        )
                        .child(
                            surfaces::danger_action(
                                "lab-forget",
                                "trash",
                                tr("Forget the lab"),
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.leave_lab(true, window, cx)),
                            ),
                        ),
                );
            }
        }
        if status.state == "active" {
            card = card.child(self.lab_knowledge_base(status, cx));
        }
        if let Some(note) = &self.lab_note {
            let (text, tone) = match note {
                Ok(said) => (said.clone(), Tone::Success),
                Err(error) => (crate::i18n::host(error), Tone::Danger),
            };
            card = card.child(div().text_xs().text_color(tone_color(tone, cx)).child(text));
        }
        card
    }
}
