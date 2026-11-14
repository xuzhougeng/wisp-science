//! Slash commands in the composer: a message that is only a command is
//! run instead of sent, and a menu lists the commands.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
};

use super::Workspace;
use crate::{
    layers::Layer,
    panel::PanelTab,
    settings::Section,
    slash::{self, Slash, Typed},
};

impl Workspace {
    /// Runs what was typed when it is a command. Returns whether it was
    /// one, in which case nothing is sent.
    pub(super) fn run_typed_slash(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match slash::parse(text) {
            None => false,
            Some(Typed::NotHere(name)) => {
                // What was typed stays, to be changed into a message.
                self.error = Some(crate::i18n::fill(
                    tr("{} is not available in this desktop yet. Type /help for what is."),
                    &[&name],
                ));
                cx.notify();
                true
            }
            Some(Typed::Run(command, rest)) => {
                self.error = None;
                self.state.save_draft(String::new());
                self.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.run_slash(command, rest, window, cx);
                true
            }
        }
    }

    pub(super) fn run_slash(
        &mut self,
        command: Slash,
        rest: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::SlashMenu);
        match command {
            Slash::Review => self.open_review(window, cx),
            Slash::Trajectory => self.open_trajectory(window, cx),
            Slash::Context => self.open_model_view(window, cx),
            Slash::Btw => {
                self.open_aside(window, cx);
                if !rest.is_empty() {
                    self.aside_input
                        .update(cx, |input, cx| input.set_value(rest, window, cx));
                    self.ask_aside(window, cx);
                }
            }
            Slash::Plan => self.toggle_plan_first(window, cx),
            Slash::SaveAsSkill => self.ask_for_skill(window, cx),
            Slash::Skills => {
                self.open_settings(window, cx);
                self.show_settings_section(Section::Skills, window, cx);
            }
            Slash::Files => self.toggle_files(window, cx),
            Slash::Upload => self.attach_files(window, cx),
            Slash::Memo => self.show_panel_tab(PanelTab::Memos, window, cx),
            Slash::Remember => self.show_panel_tab(PanelTab::Memory, window, cx),
            Slash::Archive => self.open_research_archive(window, cx),
            Slash::Help => {
                self.layers.open(Layer::SlashMenu);
                self.focus_composer(window, cx);
            }
        }
        cx.notify();
    }

    /// The "/" button of the composer, with the list of commands above it
    /// while that is open.
    pub(super) fn slash_button(&self, cx: &mut Context<Self>) -> Div {
        let open = self.layers.contains(Layer::SlashMenu);
        let button = div().relative().child(
            Button::new("slash-commands")
                .label("/")
                .small()
                .ghost()
                .selected(open)
                .tooltip(tr("Commands: type one as a message, or pick it here"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_composer_menu(Layer::SlashMenu, window, cx)
                })),
        );
        if !open {
            return button;
        }
        let mut menu = Self::composer_menu(380., cx);
        for (name, command, about) in slash::COMMANDS {
            if command == Slash::Help {
                continue;
            }
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("slash{name}")),
                    name,
                    tr(about),
                    false,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.run_slash(command, String::new(), window, cx)
                })),
            );
        }
        button.child(deferred(menu))
    }
}
