//! The first-run guide: a dialog over the first screen that says what Wisp
//! is, sets up a DeepSeek model with one key, and offers to install Python.
//! Finishing it or putting it away (Escape) marks it done for good, in this
//! desktop and the WebView one.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::ModelSource;

use super::{
    Workspace,
    shell::{error_banner, glyph},
    surfaces::{self, DialogWidth},
};
use crate::{backend::receive, layers::Layer, onboarding, settings, toolchain};

impl Workspace {
    /// Shows the guide on a first run.
    pub(super) fn check_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.onboarding_done();
        cx.spawn_in(window, async move |this, cx| {
            let done = receive(reply).await.unwrap_or(true);
            let _ = this.update(cx, |this, cx| {
                if !done {
                    this.onboarding_step = 0;
                    this.onboarding_error = None;
                    this.layers.open(Layer::Onboarding);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Shows the guide again from its first step, as the WebView's Setup
    /// command does.
    pub(super) fn open_onboarding(&mut self, cx: &mut Context<Self>) {
        self.onboarding_step = 0;
        self.onboarding_error = None;
        self.layers.open(Layer::Onboarding);
        cx.notify();
    }

    /// Whether the model step can save a key: not while models come from
    /// the environment.
    fn onboarding_model_settable(&self) -> bool {
        self.settings.current().is_none_or(|current| {
            current.source != ModelSource::Environment && current.magpie.is_none()
        })
    }

    /// Whether Magpie supplies the model, so the guide's model step has
    /// nothing to ask.
    fn onboarding_magpie(&self) -> bool {
        self.settings
            .current()
            .is_some_and(|current| current.magpie.is_some())
    }

    /// Closes the guide and remembers it was seen.
    pub(super) fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        self.layers.close(Layer::Onboarding);
        self.onboarding_error = None;
        let reply = self.backend.finish_onboarding();
        cx.background_spawn(async move {
            let _ = receive(reply).await;
        })
        .detach();
        cx.notify();
    }

    fn onboarding_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let step = self.onboarding_step;
        if step + 1 >= onboarding::STEPS {
            self.finish_onboarding(cx);
            return;
        }
        let key = self.onboarding_key.read(cx).value().trim().to_owned();
        if step == onboarding::MODEL && !key.is_empty() && self.onboarding_model_settable() {
            if self.onboarding_saving {
                return;
            }
            self.onboarding_saving = true;
            self.onboarding_error = None;
            let reply = self.backend.set_up_deepseek(key);
            cx.spawn_in(window, async move |this, cx| {
                let result = receive(reply).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.onboarding_saving = false;
                    match result {
                        Ok(_) => {
                            this.onboarding_key
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            this.onboarding_step += 1;
                            this.load_model_settings(false, window, cx);
                        }
                        Err(error) => this.onboarding_error = Some(error),
                    }
                    cx.notify();
                });
            })
            .detach();
            cx.notify();
            return;
        }
        self.onboarding_error = None;
        self.onboarding_step = step + 1;
        if self.onboarding_step == onboarding::MODEL && self.onboarding_model_settable() {
            self.onboarding_key
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn onboarding_body(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let text = |body: &'static str| div().text_sm().text_color(muted).child(body);
        match self.onboarding_step {
            0 => div().v_flex().gap_2().child(text(tr(
                "A research assistant on your own computer: plan experiments, analyse data \
                 in Python and R, search the literature and biological databases, and keep \
                 every result with the run and the conversation that made it.",
            ))),
            1 => div().v_flex().gap_2().children([
                text(tr(
                    "Each project is a folder. Conversations read and write its files, run \
                     code here or on an SSH host, and record what they made.",
                )),
                text(tr(
                    "Skills, connectors to literature and biology databases, and MCP servers \
                     widen what the model can do; you approve what changes things.",
                )),
                text(tr(
                    "Papers keep each figure with the results and runs behind it; the research \
                     journey keeps what happened day by day.",
                )),
            ]),
            onboarding::MODEL if self.onboarding_magpie() => div().v_flex().gap_2().child(text(
                tr("Magpie is connected and chooses the model; nothing to do here."),
            )),
            onboarding::MODEL if !self.onboarding_model_settable() => {
                div().v_flex().gap_2().child(text(tr(
                    "A model is set by the WISP_* variables this app was started with; nothing \
                     to do here.",
                )))
            }
            onboarding::MODEL => div()
                .v_flex()
                .gap_2()
                .child(text(tr(
                    "Open the DeepSeek console, sign in, create a key and copy it. Paste it \
                     here: deepseek-v4-flash becomes the default, with deepseek-v4-pro for \
                     heavier work. The key is kept in the system keychain, not in the project.",
                )))
                .child(
                    div().h_flex().gap_2().child(
                        Button::new("onboarding-get-key")
                            .icon(glyph("external-link"))
                            .label(tr("Open the DeepSeek console"))
                            .small()
                            .ghost()
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.open_url(onboarding::DEEPSEEK_KEYS)),
                            ),
                    ),
                )
                .child(super::surfaces::input(&self.onboarding_key, cx).small())
                .child(
                    div().h_flex().child(
                        Button::new("onboarding-other-models")
                            .label(tr("Another provider: Settings, Models"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish_onboarding(cx);
                                this.open_settings(window, cx);
                                this.show_settings_section(settings::Section::Models, window, cx);
                            })),
                    ),
                ),
            _ => {
                let mut tools = div().v_flex().gap_2().child(text(tr(
                    "Wisp can install Python (with uv), R and Node.js into its own folder, \
                     without administrator rights. Python is enough to start.",
                )));
                match &self.toolchain {
                    Some(status) if toolchain::running(status) => {
                        tools =
                            tools.child(div().text_sm().child(toolchain::progress_line(status)));
                    }
                    Some(status) => {
                        tools =
                            tools.child(div().text_sm().child(toolchain::installed_line(status)));
                        if let Some((said, _)) = toolchain::outcome(status) {
                            tools = tools.child(div().text_xs().text_color(muted).child(said));
                        }
                        if !status.installed.iter().any(|tool| tool == "python") {
                            tools = tools.child(
                                div().h_flex().child(
                                    Button::new("onboarding-install-python")
                                        .icon(glyph("download"))
                                        .label(tr("Install Python (with uv)"))
                                        .small()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toolchain_choice = vec!["python"];
                                            this.install_toolchain(window, cx);
                                        })),
                                ),
                            );
                        }
                    }
                    None => {}
                }
                tools.child(text(tr(
                    "R, Node.js and the progress of an install are in Settings, Environments.",
                )))
            }
        }
    }

    pub(super) fn render_onboarding(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let step = self.onboarding_step;
        let primary = cx.theme().primary;
        let border = cx.theme().border;
        let key_typed = !self.onboarding_key.read(cx).value().trim().is_empty();
        let mut dots = div().h_flex().gap_1().justify_center();
        for index in 0..onboarding::STEPS {
            dots = dots.child(div().size(px(6.)).rounded_full().bg(if index == step {
                primary
            } else {
                border
            }));
        }
        // Put away with Escape or by finishing; a click beside it does
        // nothing, so a stray one does not lose the guide.
        let mut card = surfaces::dialog("onboarding", DialogWidth::Narrow, onboarding::title(step))
            .gap_3()
            .child(self.onboarding_body(cx))
            .aside(dots);
        if let Some(error) = &self.onboarding_error {
            card = card.child(error_banner(error, cx));
        }
        if step > 0 {
            card = card.action(
                Button::new("onboarding-back")
                    .label(tr("Back"))
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.onboarding_step = this.onboarding_step.saturating_sub(1);
                        this.onboarding_error = None;
                        cx.notify();
                    })),
            );
        }
        card.action(
            Button::new("onboarding-next")
                .label(onboarding::next_label(
                    step,
                    key_typed,
                    self.onboarding_model_settable(),
                ))
                .small()
                .primary()
                .disabled(self.onboarding_saving)
                .on_click(cx.listener(|this, _, window, cx| this.onboarding_next(window, cx))),
        )
    }
}
