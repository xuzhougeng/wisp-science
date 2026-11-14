//! The home page: every project and the most recent conversations. Removing
//! a project asks first, in a layer that Escape closes.
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use wisp_dto::ProjectSummary;

use super::{
    Workspace, chip,
    shell::{error_banner, glyph, icon_button},
};
use crate::{
    home,
    present::{self, Tone},
};

fn section(title: &'static str, cx: &App) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .child(title)
}

/// The shared look of a project or conversation card.
fn card(id: SharedString, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .px_4()
        .py_3()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
}

impl Workspace {
    fn project_card(
        &self,
        project: &ProjectSummary,
        now: i64,
        home_dir: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = project.id.clone();
        let muted = cx.theme().muted_foreground;
        let ring = cx.theme().ring;
        let mut top = div().h_flex().gap_2().child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .child(project.name.clone()),
        );
        for (label, tone) in home::activity(project) {
            top = top.child(chip(label, tone, cx));
        }
        top = top.child(div().flex_1()).child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(muted)
                .child(home::age(now, project.updated_at)),
        );
        let card = card(SharedString::from(format!("project-{id}")), cx)
            .v_flex()
            .gap_1()
            .child(top)
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(muted)
                    .child(home::short_path(&project.workspace_dir, home_dir)),
            );
        if self.removing.as_ref() == Some(&id) {
            // Not a link while the question is open: a stray click must not
            // open the project that is about to be removed.
            return card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .pt_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(
                                "Remove this project and its conversations from Wisp? \
                                 The folder and its files stay on disk.",
                            ),
                    )
                    .child(
                        Button::new("remove-confirm")
                            .label("Remove")
                            .small()
                            .danger()
                            .disabled(self.pending)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.remove_project(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("remove-cancel")
                            .label("Cancel")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_remove_project(cx))),
                    ),
            );
        }
        let starred = project.starred;
        let star_id = id.clone();
        let mut actions = div().h_flex().gap_1().child(
            icon_button(
                SharedString::from(format!("star-{id}")),
                if starred { "star-filled" } else { "star" },
                if starred {
                    "Stop keeping first"
                } else {
                    "Keep first"
                },
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                // The card underneath opens the project; this click is not that.
                cx.stop_propagation();
                this.toggle_star(star_id.clone(), !starred, window, cx)
            })),
        );
        if wisp_server::desktop::removable(&id) {
            let remove_id = id.clone();
            actions = actions.child(
                icon_button(
                    SharedString::from(format!("remove-{id}")),
                    "trash",
                    "Remove from Wisp",
                )
                .disabled(self.pending)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.ask_remove_project(remove_id.clone(), cx)
                })),
            );
        }
        card.child(
            div()
                .h_flex()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(muted)
                        .child(home::counts(project)),
                )
                .child(actions),
        )
        .cursor_pointer()
        .hover(|style| style.border_color(ring.opacity(0.6)))
        .on_click(
            cx.listener(move |this, _, window, cx| this.enter_project(id.clone(), window, cx)),
        )
    }

    pub(super) fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let ring = cx.theme().ring;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        let home_dir = dirs::home_dir().map(|path| path.to_string_lossy().into_owned());

        let header = div()
            .h_flex()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .h_flex()
                            .gap(px(7.))
                            .text_3xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(div().text_color(cx.theme().primary).child("Wisp"))
                            .child("Science"),
                    )
                    .child(
                        div()
                            .text_color(muted)
                            .child("Rigorous research, with Wisp at your side."),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(chip("GPUI preview", Tone::Neutral, cx))
                    .child(
                        icon_button("home-search", "search", "Search everything").on_click(
                            cx.listener(|this, _, window, cx| this.toggle_palette(window, cx)),
                        ),
                    )
                    .child(icon_button("home-favorites", "star", "Favorites").on_click(
                        cx.listener(|this, _, window, cx| this.open_favorites(window, cx)),
                    ))
                    .child(icon_button("home-settings", "gear", "Settings").on_click(
                        cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                    ))
                    .child(
                        Button::new("open-project")
                            .icon(glyph("folder-plus"))
                            .label("Open folder")
                            .primary()
                            .disabled(self.pending)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_project(window, cx)),
                            ),
                    ),
            );

        let mut projects = div()
            .v_flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .child(section("Projects", cx));
        for project in &self.projects {
            projects = projects.child(self.project_card(project, now, home_dir.as_deref(), cx));
        }

        let mut recent = div()
            .v_flex()
            .gap_2()
            .flex_1()
            .min_w_0()
            .child(section("Recent conversations", cx));
        if self.recent.is_empty() {
            recent = recent.child(
                div()
                    .py_2()
                    .text_sm()
                    .text_color(muted)
                    .child("Conversations you start will appear here."),
            );
        }
        for row in &self.recent {
            let project = row.project_id.clone();
            let session = row.id.clone();
            let age = home::age(now, row.activity_at);
            let place = if row.project_name.is_empty() {
                age
            } else {
                format!("{} · {age}", row.project_name)
            };
            let mut line = card(SharedString::from(format!("recent-{session}")), cx)
                .h_flex()
                .gap_3()
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().child(row.title.clone()))
                        .child(div().truncate().text_xs().text_color(muted).child(place)),
                );
            if let Some((label, tone)) = present::session_status(&row.status) {
                line = line.child(chip(label, tone, cx));
            }
            recent = recent.child(
                line.cursor_pointer()
                    .hover(|style| style.border_color(ring.opacity(0.6)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_session(project.clone(), session.clone(), window, cx)
                    })),
            );
        }

        let mut page = div()
            .v_flex()
            .w_full()
            .max_w(px(1080.))
            .mx_auto()
            .px_8()
            .py_10()
            .gap_8()
            .child(header);
        if !self.settings.configured() {
            page = page.child(
                div()
                    .h_flex()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .rounded(cx.theme().radius_lg)
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .child("Choose a model before you start: Wisp needs one to answer."),
                    )
                    .child(
                        Button::new("home-configure-model")
                            .label("Open Settings")
                            .small()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    ),
            );
        }
        if let Some(error) = &self.error {
            page = page.child(error_banner(error, cx));
        }
        div().id("home").size_full().overflow_y_scroll().child(
            page.child(
                div()
                    .h_flex()
                    .items_start()
                    .gap_8()
                    .child(projects)
                    .child(recent),
            ),
        )
    }
}
