//! A console on one of the open conversation's interpreters, over the
//! composer: the variables it holds, code of the researcher's own to run
//! in it with what that printed and drew, and Restart.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::RuntimeLanguage;

use super::{Workspace, shell::icon_button, surfaces};
use crate::{backend::receive, console, layers::Layer};

impl Workspace {
    /// Starts the conversation's Python or R, if it is not running, and
    /// opens the console on it.
    pub(super) fn start_console(
        &mut self,
        language: RuntimeLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::Palette);
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            self.error = Some(tr("Open a conversation first.").to_owned());
            cx.notify();
            return;
        };
        self.error = None;
        let reply = self.backend.start_runtime(project, session, language);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(id) => {
                        this.load_status(window, cx);
                        this.open_console(id, window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the console on interpreter `id` and reads what it holds.
    pub(super) fn open_console(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.console_runtime.as_deref() != Some(id.as_str()) {
            self.console_entries.clear();
        }
        self.console_runtime = Some(id);
        self.console_objects = None;
        self.console_scroll.scroll_to_bottom();
        self.layers.open(Layer::RuntimeConsole);
        self.console_input
            .update(cx, |input, cx| input.focus(window, cx));
        self.load_console_objects(window, cx);
        cx.notify();
    }

    /// Shows a run made from outside the console (a script's editor) in
    /// the console on interpreter `id`, opening it there without taking
    /// the keyboard from where the run was made.
    pub(super) fn show_console_run(
        &mut self,
        id: String,
        code: &str,
        output: wisp_server::desktop::RuntimeOutput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.console_runtime.as_deref() != Some(id.as_str()) {
            self.console_entries.clear();
        }
        self.console_runtime = Some(id);
        // Under what is open, so Escape still closes the editor first.
        self.layers.open_beneath(Layer::RuntimeConsole);
        console::push(&mut self.console_entries, console::Entry::new(code, output));
        self.console_scroll.scroll_to_bottom();
        self.load_console_objects(window, cx);
        self.load_status(window, cx);
        cx.notify();
    }

    fn close_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::RuntimeConsole) {
            self.console_runtime = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    fn load_console_objects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session), Some(id)) = (
            self.state.project.clone(),
            self.state.session.clone(),
            self.console_runtime.clone(),
        ) else {
            return;
        };
        let reply = self.backend.runtime_objects(project, session, id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.console_runtime.as_deref() == Some(id.as_str()) {
                    // An interpreter that cannot say lists nothing.
                    this.console_objects = Some(result.unwrap_or_default());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_in_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session), Some(id)) = (
            self.state.project.clone(),
            self.state.session.clone(),
            self.console_runtime.clone(),
        ) else {
            return;
        };
        let code = self.console_input.read(cx).value().to_string();
        if code.trim().is_empty() || self.console_busy {
            return;
        }
        self.console_busy = true;
        self.error = None;
        let reply = self
            .backend
            .run_in_runtime(project, session, id, code.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.console_busy = false;
                match result {
                    Ok(output) => {
                        console::push(
                            &mut this.console_entries,
                            console::Entry::new(&code, output),
                        );
                        this.console_scroll.scroll_to_bottom();
                        this.console_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.load_console_objects(window, cx);
                        this.load_status(window, cx);
                    }
                    // What was typed stays, to be put right.
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn restart_console_runtime(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session), Some(id)) = (
            self.state.project.clone(),
            self.state.session.clone(),
            self.console_runtime.clone(),
        ) else {
            return;
        };
        self.console_busy = true;
        let reply = self.backend.restart_runtime(project, session, id);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.console_busy = false;
                match result {
                    // The new interpreter has an id of its own.
                    Ok(id) => {
                        this.console_entries.clear();
                        this.console_runtime = Some(id);
                        this.console_objects = None;
                        this.load_console_objects(window, cx);
                        this.load_status(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_console(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !self.layers.contains(Layer::RuntimeConsole) {
            return None;
        }
        let id = self.console_runtime.as_ref()?;
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let label = self
            .status
            .as_ref()
            .and_then(|status| status.runtimes.iter().find(|row| &row.id == id))
            .map(crate::controls::runtime_label)
            .unwrap_or_else(|| tr("Interpreter").to_owned());
        let busy = self.console_busy;
        let mut panel = surfaces::bar("runtime-console", cx)
            .v_flex()
            .gap_2()
            .max_h(px(420.))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(label))
                    .child(
                        Button::new("console-restart")
                            .label(tr("Restart"))
                            .xsmall()
                            .ghost()
                            .disabled(busy)
                            .tooltip(tr("Start it afresh: what it holds in memory is lost"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.restart_console_runtime(window, cx)
                            })),
                    )
                    .child(
                        icon_button("console-close", "close", "Close the console").on_click(
                            cx.listener(|this, _, window, cx| this.close_console(window, cx)),
                        ),
                    ),
            );
        let mut variables = div()
            .v_flex()
            .gap_0p5()
            .child(div().text_xs().text_color(muted).child(tr("Variables")));
        match &self.console_objects {
            None => {
                variables =
                    variables.child(div().text_xs().text_color(muted).child(tr("Loading…")));
            }
            Some(list) if list.objects.is_empty() => {
                variables =
                    variables.child(div().text_xs().text_color(muted).child(tr("None yet")));
            }
            Some(list) => {
                for object in list.objects.iter().take(40) {
                    variables = variables.child(
                        div()
                            .text_xs()
                            .font_family(mono.clone())
                            .truncate()
                            .child(console::object_line(object)),
                    );
                }
                if list.total_count > list.objects.len().min(40) {
                    variables = variables.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(crate::i18n::fill(tr("{} in all"), &[&list.total_count])),
                    );
                }
            }
        }
        let mut runs = div()
            .id("console-runs")
            .v_flex()
            .gap_2()
            .max_h(px(220.))
            .overflow_y_scroll()
            .track_scroll(&self.console_scroll);
        for entry in &self.console_entries {
            let mut shown = div().v_flex().gap_1().child(
                div()
                    .text_xs()
                    .font_family(mono.clone())
                    .text_color(muted)
                    .child(format!("> {}", entry.code)),
            );
            if !entry.text.is_empty() {
                shown = shown.child(
                    div()
                        .text_xs()
                        .font_family(mono.clone())
                        .child(entry.text.clone()),
                );
            }
            for plot in &entry.plots {
                shown = shown.child(
                    img(ImageSource::Image(plot.clone()))
                        .max_w(px(360.))
                        .max_h(px(240.)),
                );
            }
            runs = runs.child(shown);
        }
        panel = panel.child(variables).child(runs).child(
            div()
                .h_flex()
                .items_end()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .h(px(64.))
                        .child(Input::new(&self.console_input).h_full()),
                )
                .child(
                    Button::new("console-run")
                        .label(if busy { tr("Running…") } else { tr("Run") })
                        .small()
                        .disabled(busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.run_in_console(window, cx)),
                        ),
                ),
        );
        Some(panel)
    }
}
