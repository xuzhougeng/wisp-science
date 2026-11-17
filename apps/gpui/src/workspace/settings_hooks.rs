//! The Hooks page of Settings: your shell commands that run at points of a
//! turn, and the open project's `.wisp/hooks.json` to review and trust. The
//! form is a layer inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_dto::{CommandHook, HookEvent, ProjectHooks};

use super::{
    Workspace,
    settings_sections::setting,
    shell::error_banner,
    surfaces::{self, RecordRow, switch},
};
use crate::{
    assets::icon,
    backend::{Reply, receive},
    layers::Layer,
    settings,
};

impl Workspace {
    pub(super) fn load_hooks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.command_hooks();
        self.keep_hooks(reply, window, cx);
        self.project_hook_file = None;
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let reply = self.backend.project_hooks(project);
        self.keep_project_hooks(reply, window, cx);
    }

    /// Shows the hooks the host kept, and closes the form when it took them.
    fn keep_hooks(
        &mut self,
        reply: Reply<Vec<CommandHook>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(hooks) => {
                        this.command_hooks = hooks;
                        this.layers.close(Layer::HookForm);
                        this.hook_editing = None;
                    }
                    // The form stays open with what was typed.
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn keep_project_hooks(
        &mut self,
        reply: Reply<Option<ProjectHooks>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(file) => this.project_hook_file = file,
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves the list with one change made to it.
    fn save_hooks(
        &mut self,
        change: impl FnOnce(&mut Vec<CommandHook>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut hooks = self.command_hooks.clone();
        change(&mut hooks);
        let reply = self.backend.set_command_hooks(hooks);
        self.keep_hooks(reply, window, cx);
    }

    /// Opens the form on a new hook, or on the one at `index` to change it.
    fn edit_hook(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let shown = index.and_then(|at| self.command_hooks.get(at)).cloned();
        let event = shown
            .as_ref()
            .map_or(HookEvent::PreToolUse, |hook| hook.event);
        self.hook_editing = Some((index, event));
        self.layers.open(Layer::HookForm);
        self.settings_error = None;
        let values = match &shown {
            Some(hook) => [
                hook.matcher.clone(),
                hook.command.clone(),
                hook.timeout
                    .map(|secs| secs.to_string())
                    .unwrap_or_default(),
            ],
            None => Default::default(),
        };
        for (field, value) in self.hook_fields.iter().zip(values) {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.hook_fields[1].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn save_hook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((index, event)) = self.hook_editing else {
            return;
        };
        let [matcher, command, timeout] =
            [0, 1, 2].map(|at| self.hook_fields[at].read(cx).value().to_string());
        if command.trim().is_empty() {
            self.settings_error = Some(tr("Write the command the hook runs").into());
            return cx.notify();
        }
        let timeout = match settings::hook_timeout(&timeout) {
            Ok(timeout) => timeout,
            Err(problem) => {
                self.settings_error = Some(problem);
                return cx.notify();
            }
        };
        let enabled = index
            .and_then(|at| self.command_hooks.get(at))
            .is_none_or(|hook| hook.enabled);
        let hook = CommandHook {
            event,
            matcher: if event.matches_tools() {
                matcher
            } else {
                String::new()
            },
            command,
            enabled,
            timeout,
        };
        self.save_hooks(
            move |hooks| match index.filter(|at| *at < hooks.len()) {
                Some(at) => hooks[at] = hook,
                None => hooks.push(hook),
            },
            window,
            cx,
        );
    }

    fn trust_project_hooks(
        &mut self,
        sha256: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        self.settings_error = None;
        let reply = self.backend.set_project_hooks_trust(project, sha256);
        self.keep_project_hooks(reply, window, cx);
    }

    /// The hook being added or changed, under the list: one setting row
    /// for each field.
    fn hook_form(&self, cx: &mut Context<Self>) -> Div {
        let Some((index, chosen)) = self.hook_editing else {
            return div();
        };
        let mut events = div().h_flex().flex_wrap().justify_end().gap_1();
        for (event, _) in settings::HOOK_EVENTS {
            events = events.child(
                surfaces::choice_chip(
                    SharedString::from(format!("hook-event-{}", event.as_str())),
                    event == chosen,
                    cx,
                )
                .child(event.as_str())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some((_, current)) = &mut this.hook_editing {
                        *current = event;
                    }
                    cx.notify();
                })),
            );
        }
        let about = settings::HOOK_EVENTS
            .iter()
            .find(|(event, _)| *event == chosen)
            .map_or("", |(_, about)| *about);
        let field = |input: &Entity<gpui_component::input::InputState>, width: f32| {
            div().w(px(width)).child(surfaces::input(input, cx).small())
        };
        let mut form = div()
            .v_flex()
            .child(surfaces::settings_heading(
                if index.is_some() {
                    tr("Change")
                } else {
                    tr("Add a hook")
                },
                cx,
            ))
            .child(setting(tr("When it runs"), tr(about), events, cx));
        if chosen.matches_tools() {
            form = form.child(setting(
                tr("Tools"),
                tr("A regular expression of their names; empty is every tool."),
                field(&self.hook_fields[0], 300.),
                cx,
            ));
        }
        form.child(setting(
            tr("Command"),
            tr("Run in the project folder with the event as JSON on stdin."),
            field(&self.hook_fields[1], 300.),
            cx,
        ))
        .child(setting(
            tr("Timeout"),
            tr("In seconds; empty is a minute."),
            field(&self.hook_fields[2], 120.),
            cx,
        ))
        .child(
            div()
                .h_flex()
                .gap_2()
                .pt_3()
                .child(
                    Button::new("hook-save")
                        .label(tr("Save"))
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| this.save_hook(window, cx))),
                )
                .child(
                    Button::new("hook-cancel")
                        .label(tr("Cancel"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::HookForm);
                            this.hook_editing = None;
                            cx.notify();
                        })),
                ),
        )
    }

    /// One hook as a record row: its command, when it runs after it; a
    /// switch, Change and Remove (last, in the danger colour) at its end.
    fn hook_row(&self, index: usize, hook: &CommandHook, cx: &mut Context<Self>) -> RecordRow {
        let on = hook.enabled;
        surfaces::record_row(
            SharedString::from(format!("hook-{index}")),
            hook.command.clone(),
        )
        .icon("terminal")
        .mono()
        .suffix(settings::hook_detail(hook))
        .when(!on, |row| row.opacity(0.6))
        .action(
            switch(
                SharedString::from(format!("hook-on-{index}")),
                on,
                tr("Run this hook"),
            )
            .on_change(cx.listener(move |this, _, window, cx| {
                this.save_hooks(
                    move |hooks| {
                        if let Some(hook) = hooks.get_mut(index) {
                            hook.enabled = !on;
                        }
                    },
                    window,
                    cx,
                )
            })),
        )
        .action(
            surfaces::icon_action(
                SharedString::from(format!("hook-edit-{index}")),
                "edit",
                tr("Change"),
                surfaces::IconTone::Faded,
                cx,
            )
            .on_click(
                cx.listener(move |this, _, window, cx| this.edit_hook(Some(index), window, cx)),
            ),
        )
        .action(
            surfaces::danger_action(
                SharedString::from(format!("hook-remove-{index}")),
                "trash",
                tr("Remove"),
                cx,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.save_hooks(
                    move |hooks| {
                        if index < hooks.len() {
                            hooks.remove(index);
                        }
                    },
                    window,
                    cx,
                )
            })),
        )
    }

    /// The open project's hooks file, to review and trust.
    fn project_hooks_section(&self, cx: &mut Context<Self>) -> Div {
        let mut section = div().v_flex().gap_2().child(surfaces::settings_heading(
            tr("This project's hooks file"),
            cx,
        ));
        if self.state.project.is_none() {
            return section.child(surfaces::settings_note(
                tr("Open a project to review its .wisp/hooks.json."),
                cx,
            ));
        }
        let Some(file) = &self.project_hook_file else {
            return section.child(surfaces::settings_note_help(tr("This project has no .wisp/hooks.json."),
                tr(
                    "This project has no .wisp/hooks.json. One in Claude Code's layout runs once you trust it here.",
                ),
                cx,
            ));
        };
        let sha = file.sha256.clone();
        let button = if file.trusted {
            surfaces::outlined_action("project-hooks-untrust", "ban", tr("Stop trusting it"), cx)
                .on_click(
                    cx.listener(|this, _, window, cx| this.trust_project_hooks(None, window, cx)),
                )
        } else {
            surfaces::outlined_action(
                "project-hooks-trust",
                "shield",
                tr("Trust it as it is now"),
                cx,
            )
            .disabled(file.error.is_some())
            .tooltip(tr(
                "Its commands run in this project's turns until the file changes",
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.trust_project_hooks(Some(sha.clone()), window, cx)
            }))
        };
        section = section.child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(surfaces::read_only(file.path.clone(), cx).flex_1())
                .child(if file.trusted {
                    surfaces::stamp(surfaces::StampKind::Success, tr("Trusted"), cx)
                } else {
                    surfaces::stamp(
                        surfaces::StampKind::Waiting,
                        tr("Not trusted: it does not run"),
                        cx,
                    )
                })
                .child(button),
        );
        if let Some(error) = &file.error {
            section = section.child(error_banner(error, cx));
        }
        let rows: Vec<_> = file
            .hooks
            .iter()
            .enumerate()
            .map(|(index, hook)| {
                surfaces::record_row(
                    SharedString::from(format!("project-hook-{index}")),
                    hook.command.clone(),
                )
                .icon("terminal")
                .mono()
                .suffix(settings::hook_detail(hook))
            })
            .collect();
        if !rows.is_empty() {
            section = section.child(surfaces::record_group(rows, cx));
        }
        section
    }

    pub(super) fn render_hook_settings(&self, cx: &mut Context<Self>) -> Div {
        let mut page = self.settings_page(cx).child(
            surfaces::settings_note_help(
                tr("Shell commands that run at points of a turn."),
                tr(
                    "Shell commands that run at points of a turn, with the event as JSON on stdin, \
                 as in Claude Code and Codex. Exit 2 blocks; a JSON answer can block, ask or add \
                 context. The built-in hooks, automatic review and tool-failure analysis, are on \
                 the Conversation page.",
                ),
                cx,
            )
            .pb_3(),
        );
        let form_open = self.layers.contains(Layer::HookForm);
        if self.command_hooks.is_empty() {
            page = page.child(surfaces::settings_note(tr("None yet."), cx).pb_2());
        }
        let rows: Vec<_> = self
            .command_hooks
            .iter()
            .enumerate()
            .map(|(index, hook)| self.hook_row(index, hook, cx))
            .collect();
        if !rows.is_empty() {
            page = page.child(surfaces::record_group(rows, cx));
        }
        page =
            if form_open {
                page.child(self.hook_form(cx))
            } else {
                page.child(div().pt_3().h_flex().child(
                    surfaces::outlined_action("hook-add", "plus", tr("Add a hook"), cx).on_click(
                        cx.listener(|this, _, window, cx| this.edit_hook(None, window, cx)),
                    ),
                ))
            };
        page.child(self.project_hooks_section(cx))
    }

    /// What went wrong with hooks in the open conversation's latest turn,
    /// over the composer; nothing when nothing did.
    pub(super) fn render_hook_failures(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let failures = &self.status.as_ref()?.hook_failures;
        if failures.is_empty() {
            return None;
        }
        let attention = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let mut banner = surfaces::bar("hook-failures", cx).v_flex().gap_1().child(
            div()
                .h_flex()
                .gap_2()
                .child(icon("circle-alert").text_color(attention))
                .child(div().child(tr("A hook did not go as written"))),
        );
        // What each said is detail: a caption under the heading.
        for failure in failures.iter().take(3) {
            banner = banner.child(div().text_xs().truncate().child(failure.clone()));
        }
        if failures.len() > 3 {
            banner = banner.child(div().text_xs().text_color(muted).child(crate::i18n::fill(
                tr("and {} more"),
                &[&(failures.len() - 3)],
            )));
        }
        Some(banner)
    }
}
