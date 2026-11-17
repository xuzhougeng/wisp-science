//! The pet: a small view of its own in the corner of the conversation,
//! so its animation redraws only itself; and its page in Settings.
use std::time::Duration;

use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_server::desktop::{Pet, PetLook};

use super::{Workspace, settings_sections::explained_setting, surfaces::switch};
use crate::{
    backend::{Reply, receive},
    pet::{self, Activity, CELL, COLUMNS, Player, ROWS},
};

/// The animated pet. It asks the workspace what the conversation is
/// doing on every frame, so nothing has to tell it.
pub(super) struct PetView {
    look: Option<PetLook>,
    player: Player,
    workspace: WeakEntity<Workspace>,
    /// The frame clock; kept only while a pet is shown.
    clock: Option<Task<()>>,
}

impl PetView {
    pub(super) fn new(workspace: WeakEntity<Workspace>) -> Self {
        Self {
            look: None,
            player: Player::default(),
            workspace,
            clock: None,
        }
    }

    /// Shows `look`, or nothing; the clock runs only while there is a pet.
    pub(super) fn show(&mut self, look: Option<PetLook>, cx: &mut Context<Self>) {
        if look.is_none() {
            self.clock = None;
        } else if self.clock.is_none() {
            self.clock = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(pet::FRAME_MS))
                        .await;
                    let alive = this.update(cx, |this, cx| {
                        let Some(look) = &this.look else {
                            return;
                        };
                        let now = this
                            .workspace
                            .read_with(cx, |workspace, _| workspace.pet_activity())
                            .unwrap_or_default();
                        this.player.observe(now);
                        if this.player.tick(&look.frames) {
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }));
        }
        self.look = look;
        cx.notify();
    }
}

impl Render for PetView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(look) = &self.look else {
            return div().into_any_element();
        };
        let (left, top) = self.player.offset();
        let name = look.name.clone();
        let doing = tr(self.player.mood().label());
        div()
            .id("pet")
            .relative()
            .w(px(CELL.0))
            .h(px(CELL.1))
            .overflow_hidden()
            .cursor_pointer()
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(format!("{name}: {doing}")).build(window, cx)
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.player.poke();
                cx.notify();
            }))
            .child(
                // The whole sheet, moved so that one frame shows.
                img(look.sheet.clone())
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(CELL.0 * COLUMNS as f32))
                    .h(px(CELL.1 * ROWS as f32)),
            )
            .into_any_element()
    }
}

impl Workspace {
    /// What the conversation on screen is doing, for the pet.
    pub(super) fn pet_activity(&self) -> Activity {
        let snapshot = self.state.snapshot.as_ref();
        // Only a turn is work: waiting for the host to create, rename or
        // rewind something is not, and would end in a jump.
        let running = snapshot.is_some_and(|s| s.running);
        Activity {
            running,
            waiting: snapshot.is_some_and(|s| !s.approvals.is_empty()),
            reviewing: self.status.as_ref().is_some_and(|status| {
                matches!(
                    status.review,
                    Some(wisp_server::desktop::ReviewOutcome::Reading)
                )
            }),
            failed: self.error.is_some(),
        }
    }

    pub(super) fn load_pet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.pet();
        self.receive_pet(reply, false, window, cx);
    }

    /// Takes the host's answer about the pet: shows it, and writes its
    /// folder into the field unless the user is typing another.
    fn receive_pet(
        &mut self,
        reply: Reply<Pet>,
        saved: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(kept) => {
                        this.pet_view
                            .update(cx, |view, cx| view.show(kept.look.clone(), cx));
                        let typed = this.pet_folder.read(cx).value().to_string();
                        let before = this.pet.as_ref().map(|pet| pet.directory.clone());
                        if saved || before.is_none_or(|before| pet::unchanged(&typed, &before)) {
                            this.pet_folder.update(cx, |input, cx| {
                                input.set_value(kept.directory.clone(), window, cx)
                            });
                        }
                        this.pet = Some(kept);
                    }
                    // A refused choice changes nothing and keeps what was typed.
                    Err(error) => this.settings_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_pet(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_error = None;
        let directory = self.pet_folder.read(cx).value().to_string();
        let reply = self.backend.set_pet(enabled, directory);
        self.receive_pet(reply, true, window, cx);
        cx.notify();
    }

    fn choose_pet_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Choose").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(picked))) = picker.await else {
                return;
            };
            let Some(folder) = picked.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let folder = folder.to_string_lossy().into_owned();
                this.pet_folder
                    .update(cx, |input, cx| input.set_value(folder, window, cx));
                // Choosing a folder keeps it; the pet stays as it was.
                let enabled = this.pet.as_ref().is_some_and(|pet| pet.enabled);
                this.save_pet(enabled, window, cx);
            });
        })
        .detach();
    }

    pub(super) fn render_pet_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let page = self.settings_page(cx);
        let Some(kept) = &self.pet else {
            return page.child(div().text_sm().text_color(muted).child(tr("Loading…")));
        };
        let enabled = kept.enabled;
        let typed = self.pet_folder.read(cx).value().to_string();
        let about = match (&kept.look, &kept.problem) {
            (Some(look), _) if look.description.is_empty() => look.name.clone(),
            (Some(look), _) => format!("{} — {}", look.name, look.description),
            (None, Some(problem)) => problem.clone(),
            (None, None) => tr("No pet configured").to_owned(),
        };
        let mut folder = div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(260.))
                    .child(super::surfaces::input(&self.pet_folder, cx).small()),
            )
            .child(
                super::surfaces::outlined_action("pet-choose", "folder", tr("Choose…"), cx)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.choose_pet_folder(window, cx)),
                    ),
            );
        if !pet::unchanged(&typed, &kept.directory) {
            folder = folder.child(
                Button::new("pet-save")
                    .icon(super::shell::glyph("save"))
                    .label(tr("Save"))
                    .small()
                    .primary()
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.save_pet(enabled, window, cx)),
                    ),
            );
        }
        page.child(
            div()
                .pb_2()
                .text_xs()
                .text_color(if kept.problem.is_some() {
                    cx.theme().danger
                } else {
                    muted
                })
                .child(about),
        )
        .child(explained_setting(
            tr("Show pet"),
            "",
            tr(
                "An animated companion in the corner of the conversation. It works while a \
                 turn runs, waits when you are needed, and jumps when the turn is done.",
            ),
            switch("pet-enabled", enabled, tr("Show pet")).on_change(
                cx.listener(move |this, _, window, cx| this.save_pet(!enabled, window, cx)),
            ),
            cx,
        ))
        .child(explained_setting(
            tr("Pet folder"),
            "",
            tr(
                "A folder holding pet.json and a sprite sheet (Codex-compatible, version 2). \
                 Nothing is read from it until the pet is turned on.",
            ),
            folder,
            cx,
        ))
    }
}
