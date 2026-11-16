//! The Hooks page of Settings: your shell commands that run at points of a
//! turn, and the open project's `.wisp/hooks.json` to review and trust. The
//! form is a layer inside Settings.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_dto::{CommandHook, HookEvent, ProjectHooks};

use super::{
    Workspace, chip,
    shell::{error_banner, glyph},
};
use crate::{
    backend::{Reply, receive},
    layers::Layer,
    present::Tone,
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

    fn hook_form(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let Some((_, chosen)) = self.hook_editing else {
            return div();
        };
        let mut events = div().h_flex().flex_wrap().gap_1();
        for (event, _) in settings::HOOK_EVENTS {
            events = events.child(
                Button::new(SharedString::from(format!("hook-event-{}", event.as_str())))
                    .label(event.as_str())
                    .small()
                    .selected(event == chosen)
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
        let mut form = div()
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(620.))
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(label(tr("When it runs")))
            .child(events)
            .child(div().text_xs().text_color(muted).child(tr(about)));
        if chosen.matches_tools() {
            form = form
                .child(label(tr(
                    "Tools, as a regular expression of their names; empty is every tool",
                )))
                .child(Input::new(&self.hook_fields[0]).small());
        }
        form.child(label(tr(
            "The command, run in the project folder with the event as JSON on stdin",
        )))
        .child(Input::new(&self.hook_fields[1]).small())
        .child(label(tr("Timeout in seconds; empty is a minute")))
        .child(
            div()
                .w(px(120.))
                .child(Input::new(&self.hook_fields[2]).small()),
        )
        .child(
            div()
                .h_flex()
                .gap_2()
                .pt_1()
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
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::HookForm);
                            this.hook_editing = None;
                            cx.notify();
                        })),
                ),
        )
    }

    fn hook_row(&self, index: usize, hook: &CommandHook, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let on = hook.enabled;
        let about = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .when(!on, |about| about.opacity(0.6))
            .child(
                div()
                    .truncate()
                    .text_sm()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(hook.command.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(settings::hook_detail(hook)),
            );
        let actions = div()
            .h_flex()
            .gap_1()
            .flex_shrink_0()
            .child(
                Button::new(SharedString::from(format!("hook-on-{index}")))
                    .label(if on { tr("On") } else { tr("Off") })
                    .small()
                    .selected(on)
                    .on_click(cx.listener(move |this, _, window, cx| {
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
            .child(
                Button::new(SharedString::from(format!("hook-edit-{index}")))
                    .icon(glyph("edit"))
                    .small()
                    .ghost()
                    .tooltip(tr("Change"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_hook(Some(index), window, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("hook-remove-{index}")))
                    .icon(glyph("trash"))
                    .small()
                    .ghost()
                    .tooltip(tr("Remove"))
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
            );
        div()
            .h_flex()
            .items_start()
            .gap_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(about)
            .child(actions)
    }

    /// The open project's hooks file, to review and trust.
    fn project_hooks_section(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut section = div().v_flex().gap_2().pt_6().child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(tr("This project's hooks file")),
        );
        if self.state.project.is_none() {
            return section.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("Open a project to review its .wisp/hooks.json.")),
            );
        }
        let Some(file) = &self.project_hook_file else {
            return section.child(div().text_xs().text_color(muted).child(tr(
                "This project has no .wisp/hooks.json. One in Claude Code's layout runs once you trust it here.",
            )));
        };
        section = section.child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(file.path.clone()),
                )
                .child(if file.trusted {
                    chip(tr("Trusted"), Tone::Success, cx)
                } else {
                    chip(tr("Not trusted: it does not run"), Tone::Attention, cx)
                }),
        );
        if let Some(error) = &file.error {
            section = section.child(error_banner(error, cx));
        }
        for hook in &file.hooks {
            section = section.child(
                div()
                    .v_flex()
                    .py_1()
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(hook.command.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(settings::hook_detail(hook)),
                    ),
            );
        }
        let sha = file.sha256.clone();
        let button = if file.trusted {
            Button::new("project-hooks-untrust")
                .label(tr("Stop trusting it"))
                .small()
                .on_click(
                    cx.listener(|this, _, window, cx| this.trust_project_hooks(None, window, cx)),
                )
        } else {
            Button::new("project-hooks-trust")
                .label(tr("Trust it as it is now"))
                .small()
                .disabled(file.error.is_some())
                .tooltip(tr(
                    "Its commands run in this project's turns until the file changes",
                ))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.trust_project_hooks(Some(sha.clone()), window, cx)
                }))
        };
        section.child(div().h_flex().child(button))
    }

    pub(super) fn render_hook_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut page =
            self.settings_page(cx)
                .child(div().pb_2().text_xs().text_color(muted).child(tr(
                    "Shell commands that run at points of a turn, with the event as JSON on stdin, \
                 as in Claude Code and Codex. Exit 2 blocks; a JSON answer can block, ask or add \
                 context. The built-in hooks, automatic review and tool-failure analysis, are on \
                 the Conversation page.",
                )));
        let form_open = self.layers.contains(Layer::HookForm);
        let editing = self.hook_editing.map(|(index, _)| index);
        if self.command_hooks.is_empty() {
            page = page.child(
                div()
                    .py_3()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("None yet.")),
            );
        }
        for (index, hook) in self.command_hooks.iter().enumerate() {
            page = page.child(self.hook_row(index, hook, cx));
            if form_open && editing == Some(Some(index)) {
                page = page.child(div().py_2().child(self.hook_form(cx)));
            }
        }
        page = if form_open && editing == Some(None) {
            page.child(div().py_2().child(self.hook_form(cx)))
        } else {
            page.child(
                div().pt_3().h_flex().child(
                    Button::new("hook-add")
                        .icon(glyph("plus"))
                        .label(tr("Add a hook"))
                        .small()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.edit_hook(None, window, cx)),
                        ),
                ),
            )
        };
        page.child(self.project_hooks_section(cx))
    }

    /// What went wrong with hooks in the open conversation's latest turn,
    /// over the composer; nothing when nothing did.
    pub(super) fn render_hook_failures(&self, cx: &mut Context<Self>) -> Option<Div> {
        let failures = &self.status.as_ref()?.hook_failures;
        if failures.is_empty() {
            return None;
        }
        let attention = cx.theme().warning;
        let mut banner = div()
            .v_flex()
            .gap_1()
            .mx_4()
            .mb_2()
            .px_3()
            .py_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(attention.opacity(0.5))
            .bg(attention.opacity(0.06))
            .text_xs()
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("A hook did not go as written")),
            );
        for failure in failures.iter().take(3) {
            banner = banner.child(div().truncate().child(failure.clone()));
        }
        if failures.len() > 3 {
            banner = banner.child(crate::i18n::fill(
                tr("and {} more"),
                &[&(failures.len() - 3)],
            ));
        }
        Some(banner)
    }
}
