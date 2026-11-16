//! Favorites: what was saved from conversations. A star under a message
//! saves it; the side panel's Highlights tab lists what this conversation
//! saved; the Favorites page, a layer shown in place of the window like
//! Settings, lists everything from every project.
use crate::i18n::tr;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
    text::TextView,
};
use wisp_server::desktop::{Favorite, LibraryItem};

use super::{
    Workspace,
    shell::{error_banner, glyph, icon_button, nav_row},
};
use crate::{
    assets::icon,
    backend::{Reply, receive},
    favorites::{self, Filter},
    home,
    layers::Layer,
    memos,
};

pub(super) fn image_format(content_type: Option<&str>) -> Option<ImageFormat> {
    Some(match content_type? {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::Webp,
        "image/svg+xml" => ImageFormat::Svg,
        "image/bmp" => ImageFormat::Bmp,
        _ => return None,
    })
}

impl Workspace {
    /// Reads what the open conversation saved, for its stars and the
    /// Highlights tab.
    pub(super) fn load_saved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            self.saved.clear();
            return;
        };
        self.saved_request += 1;
        let request = self.saved_request;
        let reply = self.backend.conversation_favorites(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.current(&selection) || this.saved_request != request {
                    return;
                }
                // A conversation that is being created has nothing saved yet;
                // its refusal is not worth a banner.
                this.saved = result.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs one change to the favorites, then reads again whatever shows
    /// them.
    pub(super) fn change_favorites<T: 'static>(
        &mut self,
        reply: Reply<T>,
        then: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(value) => {
                        then(this, value, window, cx);
                        this.load_saved(window, cx);
                        if this.layers.contains(Layer::Favorites) {
                            this.load_favorites(window, cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves the message at `index`, or takes it out if it is saved.
    pub(super) fn star_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.rows.get(index).map(|row| row.text.clone()) else {
            return;
        };
        if let Some(id) = favorites::saved_id(&self.saved, &text).map(str::to_owned) {
            return self.unstar(id, window, cx);
        }
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let reply = self.backend.star_text(project, session, text);
        self.change_favorites(
            reply,
            |this, item, _, _| this.saved.insert(0, item),
            window,
            cx,
        );
    }

    pub(super) fn unstar(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.unstar(id.clone());
        self.change_favorites(
            reply,
            move |this, (), _, _| {
                this.saved.retain(|item| item.id != id);
                if this
                    .favorite
                    .as_ref()
                    .is_some_and(|open| open.item.id == id)
                {
                    this.favorite = None;
                    this.favorite_image = None;
                }
                this.layers.close(Layer::ConfirmRemoveFavorite);
            },
            window,
            cx,
        );
    }

    /// Saves a copy of an image the conversation `session` produced.
    pub(super) fn star_artifact(
        &mut self,
        session: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let reply = self.backend.star_figure(project, session, path);
        self.change_favorites(
            reply,
            |this, item: LibraryItem, _, _| {
                this.notice = Some(crate::i18n::fill(
                    tr("Saved {} to your favorites."),
                    &[&item.title],
                ));
            },
            window,
            cx,
        );
    }

    fn use_saved(&mut self, body: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = memos::insert(&self.composer.read(cx).value(), body);
        self.state.save_draft(text.clone());
        self.fill_composer(text, window, cx);
        cx.notify();
    }

    /// What this conversation saved, newest first.
    pub(super) fn render_highlights_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let mut list = div()
            .id("highlights")
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .p_3()
            .overflow_y_scroll()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(muted)
                            .child(tr(
                                "What you saved from this conversation. Everything you saved, \
                         from every project, is in Favorites.",
                            )),
                    )
                    .child(
                        Button::new("highlights-all")
                            .icon(glyph("star"))
                            .label(tr("Favorites"))
                            .small()
                            .flex_shrink_0()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_favorites(window, cx)),
                            ),
                    ),
            );
        if let Some(error) = &self.error {
            list = list.child(error_banner(error, cx));
        }
        if self.saved.is_empty() {
            list = list.child(div().text_sm().text_color(muted).child(
                if self.state.session.is_some() {
                    tr("Nothing saved yet. Press the star under a message to keep it.")
                } else {
                    tr("Open a conversation to see what was saved from it.")
                },
            ));
        }
        for (index, item) in self.saved.iter().enumerate() {
            let (using, copying, removing, opening) = (
                item.code.clone(),
                item.code.clone(),
                item.id.clone(),
                item.id.clone(),
            );
            let words = favorites::has_text(&item.kind);
            let mut actions = div().h_flex().gap_0p5();
            if words {
                actions = actions.child(
                    Button::new(("saved-use", index))
                        .icon(glyph("arrow-right"))
                        .label(tr("Use"))
                        .xsmall()
                        .ghost()
                        .tooltip(tr("Put this into the message box"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.use_saved(&using, window, cx)
                        })),
                );
            }
            actions = actions.child(div().flex_1());
            if words {
                actions = actions.child(
                    icon_button(("saved-copy", index), "copy", tr("Copy"))
                        .xsmall()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copying.clone()))
                        })),
                );
            }
            actions = actions
                .child(
                    icon_button(("saved-open", index), "expand", tr("Open in Favorites"))
                        .xsmall()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_favorites(window, cx);
                            this.open_favorite(opening.clone(), window, cx);
                        })),
                )
                .child(
                    icon_button(
                        ("saved-remove", index),
                        "star-filled",
                        tr("Remove from favorites"),
                    )
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.unstar(removing.clone(), window, cx)
                    })),
                );
            list = list.child(
                div()
                    .v_flex()
                    .gap_1()
                    .p_2()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .h_flex()
                            .gap_1()
                            .child(icon(favorites::icon(&item.kind)).text_color(muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(item.title.clone()),
                            ),
                    )
                    .child(div().text_xs().text_color(muted).child(if words {
                        crate::memory::preview(&item.code, 220)
                    } else {
                        item.source_path.clone().unwrap_or_default()
                    }))
                    .child(actions),
            );
        }
        list.into_any_element()
    }

    pub(super) fn open_favorites(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::Palette);
        self.layers.open(Layer::Favorites);
        self.error = None;
        self.load_favorites(window, cx);
        self.favorites_search
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close_favorites(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(Layer::Favorites) {
            self.favorite = None;
            self.favorite_image = None;
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    /// Lists the favorites that match the search box and the kind chosen.
    /// Only the latest request's answer is shown.
    pub(super) fn load_favorites(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.favorites_request += 1;
        let request = self.favorites_request;
        let query = self.favorites_search.read(cx).value().to_string();
        let reply = self.backend.favorites(query, self.favorites_filter.kind());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.favorites_request != request {
                    return;
                }
                match result {
                    Ok(items) => this.favorites = Some(items),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_favorites_filter(
        &mut self,
        filter: Filter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.favorites_filter != filter {
            self.favorites_filter = filter;
            self.load_favorites(window, cx);
            cx.notify();
        }
    }

    /// Reads one item in full and shows it beside the list.
    fn open_favorite(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmRemoveFavorite);
        self.favorite_request += 1;
        let request = self.favorite_request;
        self.error = None;
        let reply = self.backend.favorite(id);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if this.favorite_request != request {
                    return;
                }
                match result {
                    Ok(favorite) => {
                        let Favorite { item, image } = &favorite;
                        this.favorite_image = image_format(item.content_type.as_deref())
                            .zip(image.clone())
                            .map(|(format, bytes)| Arc::new(Image::from_bytes(format, bytes)));
                        this.favorite = Some(favorite);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Goes to the conversation an item was saved from.
    fn open_favorite_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.favorite.as_ref().map(|open| &open.item) else {
            return;
        };
        let (project, session) = (
            item.source_project_id.clone(),
            item.source_session_id.clone(),
        );
        self.close_favorites(window, cx);
        self.layers.close(Layer::Settings);
        self.open_session(project, session, window, cx);
    }

    fn favorite_detail(&self, open: &Favorite, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let item = &open.item;
        let here = self
            .projects
            .iter()
            .any(|project| project.id == item.source_project_id);
        let source = favorites::source_line(&item.source_project_name, &item.source_session_title);
        let words = favorites::has_text(&item.kind);
        let mut actions = div().h_flex().flex_wrap().gap_2();
        if words {
            let copying = item.code.clone();
            actions = actions.child(
                Button::new("favorite-copy")
                    .icon(glyph("copy"))
                    .label(tr("Copy"))
                    .small()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copying.clone()))
                    })),
            );
        }
        actions = actions.child(
            Button::new("favorite-source")
                .icon(glyph("chat"))
                .label(tr("Open the conversation"))
                .small()
                .disabled(!here)
                .tooltip(if here {
                    tr("Go to the conversation this was saved from")
                } else {
                    tr("The project this came from is no longer in Wisp")
                })
                .on_click(cx.listener(|this, _, window, cx| this.open_favorite_source(window, cx))),
        );
        let removing = item.id.clone();
        actions = if self.layers.contains(Layer::ConfirmRemoveFavorite) {
            actions
                .child(
                    Button::new("favorite-remove-confirm")
                        .label(tr("Remove for good"))
                        .small()
                        .danger()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.unstar(removing.clone(), window, cx)
                        })),
                )
                .child(
                    Button::new("favorite-remove-cancel")
                        .label(tr("Keep"))
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::ConfirmRemoveFavorite);
                            cx.notify();
                        })),
                )
        } else {
            actions.child(
                Button::new("favorite-remove")
                    .icon(glyph("trash"))
                    .label(tr("Remove"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.layers.open(Layer::ConfirmRemoveFavorite);
                        cx.notify();
                    })),
            )
        };
        let mut detail = div()
            .v_flex()
            .gap_3()
            .p_6()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::MEDIUM)
                            .child(item.title.clone()),
                    )
                    .child(div().text_xs().text_color(muted).child(format!(
                        "{} · {}",
                        favorites::kind_label(&item.kind, item.language.as_deref()),
                        source
                    ))),
            )
            .child(actions);
        if self.layers.contains(Layer::ConfirmRemoveFavorite) {
            detail = detail.child(div().text_sm().text_color(cx.theme().danger).child(tr(
                "Remove this from your favorites? The copy kept here is deleted; \
                 the conversation it came from is not touched.",
            )));
        }
        if let Some(path) = &item.source_path {
            detail = detail.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(path.clone()),
            );
        }
        if !words {
            return detail.child(match &self.favorite_image {
                Some(image) => div().child(
                    img(image.clone())
                        .max_w_full()
                        .max_h(px(560.))
                        .object_fit(ObjectFit::Contain),
                ),
                None => div()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("This picture cannot be shown here.")),
            });
        }
        detail.child(
            TextView::markdown(
                SharedString::from(format!("favorite-{}", item.id)),
                favorites::as_markdown(&item.kind, item.language.as_deref(), &item.code),
                window,
                cx,
            )
            .selectable(true),
        )
    }

    /// The Favorites page: a list on the left, the open item on the right.
    pub(super) fn render_favorites(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or_default();
        let mut filters = div().h_flex().gap_1().px_1();
        for filter in Filter::ALL {
            filters = filters.child(
                Button::new(SharedString::from(format!("favorites-{}", filter.label())))
                    .label(filter.label())
                    .xsmall()
                    .ghost()
                    .selected(self.favorites_filter == filter)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_favorites_filter(filter, window, cx)
                    })),
            );
        }
        let mut list = div()
            .id("favorites-list")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        let searching = !self.favorites_search.read(cx).value().trim().is_empty()
            || self.favorites_filter != Filter::All;
        match &self.favorites {
            None => {
                list = list.child(
                    div()
                        .p_2()
                        .text_sm()
                        .text_color(muted)
                        .child(tr("Loading…")),
                );
            }
            Some(items) if items.is_empty() => {
                list = list.child(div().p_2().text_sm().text_color(muted).child(if searching {
                    tr("Nothing saved matches.")
                } else {
                    tr(
                        "Nothing saved yet. Press the star under a message, or beside \
                     an image in Artifacts, to keep it here.",
                    )
                }));
            }
            Some(items) => {
                let accent = cx.theme().sidebar_accent;
                for item in items {
                    let id = item.id.clone();
                    let open = self
                        .favorite
                        .as_ref()
                        .is_some_and(|open| open.item.id == item.id);
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("favorite-row-{id}")))
                            .h_flex()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(cx.theme().radius)
                            .cursor_pointer()
                            .when(open, |row| row.bg(accent))
                            .hover(move |style| style.bg(accent.opacity(0.7)))
                            .child(icon(favorites::icon(&item.kind)).text_color(muted))
                            .child(
                                div()
                                    .v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().truncate().text_sm().child(item.title.clone()))
                                    .child(div().truncate().text_xs().text_color(muted).child(
                                        format!(
                                            "{} · {}",
                                            favorites::source_line(
                                                &item.source_project_name,
                                                &item.source_session_title
                                            ),
                                            home::age(now, item.created_at)
                                        ),
                                    )),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_favorite(id.clone(), window, cx)
                            })),
                    );
                }
            }
        }
        let side = div()
            .v_flex()
            .w(px(340.))
            .flex_shrink_0()
            .h_full()
            .p_2()
            .gap_1()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                nav_row("favorites-back", "arrow-left", tr("Back"), false, true, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_favorites(window, cx))),
            )
            .child(
                div()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .text_lg()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr("Favorites")),
            )
            .child(div().px_2().pb_1().text_xs().text_color(muted).child(tr(
                "Passages, code and figures you saved, as they were when you saved them.",
            )))
            .child(
                div().py_1().child(
                    Input::new(&self.favorites_search)
                        .small()
                        .prefix(icon("search").text_color(muted)),
                ),
            )
            .child(filters)
            .child(list);
        let mut body = div()
            .id("favorite-detail")
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll();
        if let Some(error) = &self.error {
            body = body.child(div().p_6().child(error_banner(error, cx)));
        }
        body = match &self.favorite {
            Some(open) => body.child(self.favorite_detail(open, window, cx)),
            None => body.child(
                div()
                    .p_6()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("Choose something on the left to read it.")),
            ),
        };
        div().h_flex().size_full().child(side).child(body)
    }
}
