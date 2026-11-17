//! Project files opened in the centre, as the WebView's centre tabs: a
//! document opened from a file's row in the Files tab or an artifact's row
//! gets a tab after its conversation's own and is shown in the
//! conversation's place, or beside it once split (`crate::centre` keeps
//! the tabs and the widths).
//!
//! The document is the Files tab's preview drawn in the centre, not a
//! second one: the window has one preview (the file, its picture, pages
//! or text, and the script editor with its draft), and it is shown either
//! in the Files tab or in the centre. Previewing a file in the Files tab
//! while a document is shown puts the centre back on the conversation, its
//! tab kept; showing a document takes the preview from the Files tab,
//! which goes back to its list. So an R or Python script has one editor and
//! one draft, wherever it is edited.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Selectable, StyledExt};

use super::{Workspace, shell::icon_button, surfaces};
use crate::{centre, layers::Layer};

/// What is dragged when the divider between the document and the
/// conversation moves.
pub(super) struct Divider;

/// Nothing follows the pointer while the divider is dragged: the columns
/// themselves move.
struct DividerGhost;

impl Render for DividerGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

impl Workspace {
    /// The document the centre shows for the open conversation, if one is.
    pub(super) fn centre_document(&self) -> Option<&str> {
        self.centre.shown(self.state.session.as_deref())
    }

    /// Whether the one preview is the centre's: a document is shown there,
    /// and the Files tab is not previewing.
    pub(super) fn preview_in_centre(&self) -> bool {
        self.centre_document().is_some() && !self.layers.contains(Layer::FilePreview)
    }

    /// Lets go of what the preview holds, as closing the Files tab's
    /// preview does, without its layers. An unsaved edit stays as the
    /// file's draft.
    pub(super) fn drop_preview(&mut self) {
        self.file_edit = None;
        self.files.close_preview();
        self.file_image = None;
        self.file_document = None;
        self.file_pages = None;
        self.file_problem = None;
        self.file_origin = None;
        self.file_as_text = false;
        self.file_notice = None;
        self.picture_mark = None;
        self.layers.close(Layer::PictureSelect);
        self.layers.close(Layer::FileChangedOnDisk);
    }

    /// Opens `path` in the centre of the open conversation: a tab after
    /// its others, or the tab it already has, shown in the conversation's
    /// place, or beside it when split.
    pub(super) fn open_in_centre(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        self.centre.of_mut(&session).open(&path);
        self.show_centre_document(window, cx);
        cx.notify();
    }

    /// Shows the document of `path`'s tab, or the conversation for `None`.
    pub(super) fn show_centre(
        &mut self,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        let held = self.preview_in_centre();
        if !self.centre.of_mut(&session).show(path.as_deref()) {
            return;
        }
        if path.is_some() {
            self.show_centre_document(window, cx);
        } else if held {
            self.drop_preview();
            self.restore_focus(window, cx);
        }
        cx.notify();
    }

    /// Puts the document the centre shows into the preview. The Files tab
    /// gives its preview up and goes back to its list; the file it was
    /// previewing, when it is this one, moves over as it is (its zoom, its
    /// edit). Anything else is read.
    fn show_centre_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.centre_document().map(str::to_owned) else {
            return;
        };
        let held = self.files.preview_path.as_deref() == Some(path.as_str());
        self.layers.close(Layer::FilePreview);
        if !held {
            self.load_file(path, false, window, cx);
        }
        // The list that was under the Files tab's preview.
        self.ensure_files(window, cx);
    }

    /// The Files tab is about to preview a file: the document the centre
    /// shows steps back to the conversation and keeps its tab.
    pub(super) fn centre_steps_back(&mut self) {
        if let Some(session) = self.state.session.clone()
            && self.centre_document().is_some()
        {
            self.centre.of_mut(&session).show(None);
        }
    }

    /// Brings back the document a conversation showed when it comes back
    /// to the window. While the Files tab previews a file, that preview
    /// stays and the conversation is shown, its tabs kept.
    pub(super) fn restore_centre(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        if self.centre_document().is_none() {
            return;
        }
        if self.layers.contains(Layer::FilePreview) {
            self.centre.of_mut(&session).show(None);
        } else {
            self.show_centre_document(window, cx);
        }
    }

    /// Reads the document the centre shows again when the preview holds
    /// another path, as after the file was renamed.
    pub(super) fn follow_centre_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.preview_in_centre() {
            return;
        }
        let Some(path) = self.centre_document().map(str::to_owned) else {
            return;
        };
        if self.files.preview_path.as_deref() != Some(path.as_str()) {
            self.load_file(path, false, window, cx);
        }
    }

    /// Leaves the preview for the conversation: the Files tab goes back to
    /// its list, a document in the centre to the conversation (the
    /// WebView's jump back to chat).
    pub(super) fn leave_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview_in_centre() {
            self.show_centre(None, window, cx);
        } else {
            self.close_preview(window, cx);
        }
    }

    /// Closes the side panel. Marking a picture in the centre is not the
    /// panel's, though its layer is drawn under the preview's, so it stays.
    pub(super) fn close_side_panel(&mut self) -> bool {
        let centre = self.preview_in_centre();
        let marking = centre && self.layers.contains(Layer::PictureSelect);
        let region = centre && self.layers.contains(Layer::PictureRegion);
        let closed = self.layers.close(Layer::FilesPanel);
        if marking {
            self.layers.open(Layer::PictureSelect);
            if region {
                self.layers.open(Layer::PictureRegion);
            }
        }
        closed
    }

    /// Splits the centre, or puts the conversation back under the
    /// document's tab. Split, the conversation and the document share the
    /// room the side panel had, as in the WebView, which closes it.
    fn toggle_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.centre.split = !self.centre.split;
        if self.centre.split && self.layers.contains(Layer::FilesPanel) {
            self.toggle_files(window, cx);
        }
        cx.notify();
    }

    /// Keeps a document shown in the centre open with the conversation
    /// beside it, after something was put in the message from it.
    pub(super) fn keep_conversation_beside(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.preview_in_centre() {
            return;
        }
        self.centre.split = true;
        if self.layers.contains(Layer::FilesPanel) {
            self.toggle_files(window, cx);
        }
    }

    fn close_centre_tab(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        let held = self.preview_in_centre();
        if self.centre.of_mut(&session).close(&path) && held {
            self.drop_preview();
            self.restore_focus(window, cx);
        }
        cx.notify();
    }

    fn close_centre_right(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        if self.centre.of_mut(&session).close_right(&path) {
            self.show_centre_document(window, cx);
        }
        cx.notify();
    }

    fn close_centre_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.state.session.clone() else {
            return;
        };
        let held = self.preview_in_centre();
        self.centre.of_mut(&session).close_all();
        if held {
            self.drop_preview();
            self.restore_focus(window, cx);
        }
        cx.notify();
    }

    fn open_centre_menu(&mut self, path: String, cx: &mut Context<Self>) {
        self.centre_menu = Some(path);
        self.layers.open(Layer::CentreTabMenu);
        cx.notify();
    }

    fn close_centre_menu(&mut self) {
        self.layers.close(Layer::CentreTabMenu);
        self.centre_menu = None;
    }

    /// The open conversation's documents as tabs, for the strip after its
    /// own: each its file's name with a close button. A right click opens
    /// what else can be closed from it.
    pub(super) fn centre_tabs(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let Some(documents) = self
            .state
            .session
            .as_deref()
            .and_then(|session| self.centre.of(session))
        else {
            return Vec::new();
        };
        let shown = documents.shown();
        let mut tabs = Vec::new();
        for (index, document) in documents.tabs().iter().enumerate() {
            let selected = shown == Some(document.path.as_str());
            let (opened, closed, asked) = (
                document.path.clone(),
                document.path.clone(),
                document.path.clone(),
            );
            let full = SharedString::from(document.path.clone());
            let mut tab = surfaces::window_tab(
                ("centre-tab", index),
                "doc",
                document.name().to_owned(),
                selected,
                cx,
            )
            .relative()
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(full.clone()).build(window, cx)
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| this.open_centre_menu(asked.clone(), cx)),
            )
            .child(
                surfaces::tab_close(("centre-tab-close", index), tr("Close tab"), cx).on_click(
                    cx.listener(move |this, _, window, cx| {
                        // Not a click on the tab underneath.
                        cx.stop_propagation();
                        this.close_centre_tab(closed.clone(), window, cx)
                    }),
                ),
            );
            if !selected {
                tab = tab.on_click(cx.listener(move |this, _, window, cx| {
                    this.show_centre(Some(opened.clone()), window, cx)
                }));
            }
            if self.layers.contains(Layer::CentreTabMenu)
                && self.centre_menu.as_deref() == Some(document.path.as_str())
            {
                tab = tab.child(deferred(self.centre_tab_menu(&document.path, cx)));
            }
            tabs.push(tab);
        }
        tabs
    }

    /// What can be closed from a document's tab, under it: the tab, the
    /// tabs to its right, every document, as the WebView's menu has them.
    fn centre_tab_menu(&self, path: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let (current, right) = (path.to_owned(), path.to_owned());
        div()
            .id("centre-tab-menu")
            .max_h(surfaces::menu_room(None, false))
            .overflow_y_scroll()
            .absolute()
            .top(px(32.))
            .left_0()
            .w(px(220.))
            .v_flex()
            .p_1()
            .occlude()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_md()
            // A click anywhere else puts it away, as the WebView's menu goes.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.close_centre_menu();
                cx.notify();
            }))
            .child(
                Self::menu_row(
                    "centre-menu-current".into(),
                    tr("Close current"),
                    "",
                    false,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_centre_menu();
                    this.close_centre_tab(current.clone(), window, cx)
                })),
            )
            .child(
                Self::menu_row(
                    "centre-menu-right".into(),
                    tr("Close tabs to the right"),
                    "",
                    false,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_centre_menu();
                    this.close_centre_right(right.clone(), window, cx)
                })),
            )
            .child(
                Self::menu_row(
                    "centre-menu-all".into(),
                    tr("Close all files"),
                    "",
                    false,
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_centre_menu();
                    this.close_centre_all(window, cx)
                })),
            )
    }

    /// The document the centre shows: the Files tab's preview, with its
    /// header, bars and actions, drawn on the page. Its header's last
    /// button splits the centre.
    pub(super) fn render_centre_document(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let split = icon_button("centre-split", "split", "Chat beside the document")
            .selected(self.centre.split)
            .on_click(cx.listener(|this, _, window, cx| this.toggle_split(window, cx)));
        div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .children(self.preview_parts(None, Some(split.into_any_element()), window, cx))
    }

    /// The centre under the conversation's tabs and header (`page`): the
    /// conversation (`chat`) while no document is shown; the document in
    /// its place; or, split, the document on the left, a divider that
    /// drags, and the conversation in a column of its own on the right, at
    /// least `centre::CHAT_MIN` wide. While split the centre keeps room for
    /// both, so the side panel gives way first, then the document.
    pub(super) fn render_centre(
        &self,
        page: Div,
        chat: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        if self.centre_document().is_none() {
            return page.child(chat);
        }
        let document = self.render_centre_document(window, cx);
        if !self.centre.split_shown(self.state.session.as_deref()) {
            return page.child(document);
        }
        let chat_width: Length = match (self.centre_width.get(), self.centre.chat_width) {
            (Some(width), wanted) => px(centre::split_widths(width, wanted).1).into(),
            (None, Some(wanted)) => px(wanted.max(centre::CHAT_MIN)).into(),
            (None, None) => relative(centre::CHAT_SHARE).into(),
        };
        // Measures the centre, for the widths, and draws again when it
        // changes.
        let measured = self.centre_width.clone();
        let weak = cx.entity().downgrade();
        let measure = canvas(
            move |bounds, window, cx| {
                let width = Some(f32::from(bounds.size.width));
                if measured.get() != width {
                    measured.set(width);
                    window.defer(cx, move |_, cx| {
                        let _ = weak.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let line = cx.theme().border;
        let teal = cx.theme().primary;
        let divider = div()
            .id("centre-divider")
            .group("centre-divider")
            .flex_none()
            .h_full()
            .w(px(centre::DIVIDER))
            .flex()
            .justify_center()
            .cursor_col_resize()
            .child(
                div()
                    .w(px(1.))
                    .h_full()
                    .bg(line)
                    .group_hover("centre-divider", move |style| style.bg(teal)),
            )
            .on_drag(Divider, |_, _, _, cx| cx.new(|_| DividerGhost));
        let row = div()
            .id("centre-split")
            .relative()
            .h_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .on_drag_move(cx.listener(|this, moved: &DragMoveEvent<Divider>, _, cx| {
                let bounds = moved.bounds;
                this.centre.chat_width = Some(centre::dragged_chat_width(
                    f32::from(bounds.origin.x),
                    f32::from(bounds.size.width),
                    f32::from(moved.event.position.x),
                ));
                cx.notify();
            }))
            .child(measure)
            .child(document)
            .child(divider)
            .child(
                chat.flex_none()
                    .h_full()
                    .w(chat_width)
                    .min_w(px(centre::CHAT_MIN)),
            );
        page.min_w(px(centre::SPLIT_MIN)).child(row)
    }
}
