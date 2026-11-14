//! The Network page of Settings: the route each kind of the app's own
//! traffic takes, and where the model is told to install packages from.
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_dto::NetworkSettings;

use super::{Workspace, settings_sections::setting};
use crate::{backend::receive, settings};

/// The fields in the order of `settings::network_form`: what each is called
/// and what it is for.
const FIELDS: [(&str, &str); 6] = [
    (
        "Model API",
        "Requests to the models you added. Applies from the next turn, and to Test connection.",
    ),
    (
        "Connectors and MCP",
        "The bundled literature and biology connectors, and MCP servers reached over HTTP. \
         Servers reconnect when this is saved.",
    ),
    (
        "Code the model runs",
        "Handed to shell commands and Python as the usual proxy variables, so pip, conda, git \
         and curl follow it.",
    ),
    (
        "Conda mirror",
        "A channel address. The model is told to prefer it when it creates an environment or \
         installs a package.",
    ),
    (
        "Python package index",
        "An index address, as pip's --index-url takes it. The model is told to prefer it.",
    ),
    (
        "CA bundle",
        "The path of a certificate file for package managers behind an inspecting proxy.",
    ),
];

impl Workspace {
    pub(super) fn load_network(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.network_saved = false;
        let reply = self.backend.network();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(saved) => this.show_network(saved, window, cx),
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Puts what the host keeps into the fields.
    fn show_network(
        &mut self,
        saved: NetworkSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (field, value) in self
            .network_fields
            .iter()
            .zip(settings::network_values(&saved))
        {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.network = Some(saved);
    }

    fn typed_network(&self, saved: &NetworkSettings, cx: &App) -> NetworkSettings {
        let typed = self
            .network_fields
            .each_ref()
            .map(|field| field.read(cx).value().to_string());
        settings::network_form(typed, saved)
    }

    fn save_network(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(saved) = &self.network else {
            return;
        };
        let reply = self.backend.set_network(self.typed_network(saved, cx));
        self.settings_error = None;
        self.network_saved = false;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(saved) => {
                        // The host trims; show what it kept.
                        this.show_network(saved, window, cx);
                        this.network_saved = true;
                    }
                    // A refused form changes nothing and keeps what was typed.
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_network_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(saved) = &self.network else {
            return page.child(div().text_sm().text_color(muted).child("Loading…"));
        };
        let typed = self.typed_network(saved, cx);
        let values = settings::network_values(&typed);
        let heading = |title: &'static str, note: &'static str| {
            div()
                .v_flex()
                .gap_1()
                .pt_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().w(px(640.)).text_xs().text_color(muted).child(note))
        };
        let mut page = page.child(heading(
            "Routes",
            "Leave a route empty to follow the system and the environment, write none to \
             connect directly, or give a proxy address (http, https or socks5) without a \
             user name or password.",
        ));
        for (index, ((title, note), field)) in FIELDS.iter().zip(&self.network_fields).enumerate() {
            if index == settings::NETWORK_ROUTES {
                page = page.child(div().pt_4().child(heading(
                    "Packages",
                    "Advice the model is given with every turn, not a rule the app enforces.",
                )));
            }
            let mut control = div()
                .v_flex()
                .gap_1()
                .w(px(240.))
                .child(Input::new(field).small());
            if index < settings::NETWORK_ROUTES {
                control = control.child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .child(settings::route_note(&values[index])),
                );
            }
            page = page.child(setting(title, *note, control, cx));
        }
        let mut actions = div().h_flex().gap_3().pt_4().child(
            Button::new("network-save")
                .label("Save")
                .small()
                .primary()
                .on_click(cx.listener(|this, _, window, cx| this.save_network(window, cx))),
        );
        if self.network_saved && &typed == saved {
            actions = actions.child(div().text_xs().text_color(muted).child("Saved"));
        }
        page.child(actions)
    }
}
