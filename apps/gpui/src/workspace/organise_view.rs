//! How a project's conversations are organised: sections, sort and grouping,
//! select mode with its batch actions, the user's groups, and sending the
//! open conversation to another project. Every surface that opens here is a
//! layer in the window's Escape stack.
use crate::i18n::tr;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Input,
};
use wisp_server::desktop::BatchAction;

use super::{
    Workspace, chip,
    shell::{glyph, icon_button},
};
use crate::{
    assets::icon,
    backend::{Reply, receive},
    home,
    layers::Layer,
    organise::{self, Arrangement, Grouping, Listed, Listing, Row, Sorting},
    present,
};

/// Name the sidebar's arrangement is saved under.
const ARRANGEMENT: &str = "sidebar";

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn caption(text: &'static str, cx: &App) -> Div {
    div()
        .px_2()
        .pt_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// The box of a menu that floats over the page. The caller places it.
fn popup(width: f32, cx: &App) -> Div {
    div()
        .absolute()
        .w(px(width))
        .v_flex()
        .p_1()
        .occlude()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .shadow_md()
}

impl Workspace {
    pub(super) fn listing(&self, cx: &App) -> Listing {
        if !self.search.read(cx).value().trim().is_empty() {
            Listing::Search
        } else if self.show_archived {
            Listing::Archive
        } else {
            Listing::Browse
        }
    }

    /// The conversation list as it is drawn, sections included.
    pub(super) fn session_rows(&self, cx: &App) -> Vec<Row> {
        // Subagents are listed under the conversation that started them,
        // the way branches are under theirs.
        let nested = organise::nested(&self.branched, &self.dispatched);
        organise::layout(
            &Listed {
                rows: &self.sessions,
                pinned: &self.pinned,
                folders: &self.folders,
                grouped: &self.grouped,
                branched: &nested,
                collapsed: &self.collapsed,
                now: unix_now(),
            },
            self.arrangement,
            self.listing(cx),
        )
    }

    pub(super) fn load_arrangement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.preference(ARRANGEMENT);
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Some(saved)) = receive(reply).await {
                let _ = this.update(cx, |this, cx| {
                    this.arrangement = Arrangement::decode(&saved);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn set_arrangement(
        &mut self,
        arrangement: Arrangement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.arrangement == arrangement {
            return;
        }
        self.arrangement = arrangement;
        let reply = self
            .backend
            .set_preference(ARRANGEMENT, arrangement.encode());
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = receive(reply).await {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Opens one of the sidebar's small surfaces in place of the others, or
    /// closes it when it is the one showing.
    fn toggle_sidebar_layer(&mut self, layer: Layer, cx: &mut Context<Self>) {
        if !self.layers.close(layer) {
            self.layers.switch(layer, &Layer::SIDEBAR);
        }
        cx.notify();
    }

    fn toggle_section(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_owned());
        }
        cx.notify();
    }

    fn toggle_select_mode(&mut self, cx: &mut Context<Self>) {
        self.picks.clear();
        for layer in Layer::SIDEBAR {
            self.layers.close(layer);
        }
        if !self.layers.close(Layer::SelectMode) {
            self.layers.open(Layer::SelectMode);
        }
        cx.notify();
    }

    fn pick(&mut self, id: &str, range: bool, cx: &mut Context<Self>) {
        if range {
            let shown = organise::visible(&self.session_rows(cx), &self.sessions);
            self.picks.extend(id, &shown);
        } else {
            self.picks.toggle(id);
        }
        cx.notify();
    }

    fn pick_all(&mut self, cx: &mut Context<Self>) {
        let shown = organise::visible(&self.session_rows(cx), &self.sessions);
        self.picks.toggle_all(&shown);
        cx.notify();
    }

    /// Runs one call about the project's conversation list, then refreshes
    /// every list it can affect. `then` runs only if the same project is
    /// still open.
    fn organise<T: 'static>(
        &mut self,
        reply: Reply<T>,
        then: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = true;
        self.error = None;
        self.notice = None;
        let project = self.state.project.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(value) => {
                        if this.state.project == project {
                            then(this, value, window, cx);
                        }
                        this.refresh_lists(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Applies `action` to the picked conversations. `verb` is how the
    /// result line names what happened to them.
    fn run_batch(
        &mut self,
        action: BatchAction,
        verb: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let picked = self.picks.ordered(&self.sessions);
        if picked.is_empty() {
            return;
        }
        let reply = self.backend.batch(project, picked, action.clone());
        self.organise(
            reply,
            move |this, report, window, cx| {
                this.notice = Some(organise::report(verb, report.done.len(), &report.refused));
                if report.done.is_empty() {
                    // Nothing happened; the picks are still the question.
                    this.layers.close(Layer::BatchGroupMenu);
                    this.layers.close(Layer::ConfirmBatchDelete);
                    return;
                }
                this.layers.close(Layer::SelectMode);
                this.picks.clear();
                for id in &report.done {
                    match action {
                        BatchAction::Delete => {
                            this.known.remove(id);
                            this.state.forget_draft(id);
                            this.chips.remove(id);
                        }
                        BatchAction::Archive | BatchAction::Restore => {
                            if let Some(row) = this.known.get_mut(id) {
                                row.shelved = action == BatchAction::Archive;
                            }
                        }
                        BatchAction::Group(_) => {}
                    }
                }
                let gone = matches!(action, BatchAction::Delete | BatchAction::Archive);
                let open = this.state.session.as_ref();
                if gone && open.is_some_and(|open| report.done.contains(open)) {
                    this.leave_conversation(window, cx);
                }
            },
            window,
            cx,
        );
    }

    /// Opens the name field for a new group, or for `folder` to rename it.
    fn begin_folder_name(
        &mut self,
        folder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = folder
            .as_ref()
            .and_then(|id| self.folders.iter().find(|folder| &folder.id == id))
            .map(|folder| folder.name.clone())
            .unwrap_or_default();
        self.folder_target = folder;
        self.layers.close(Layer::GroupMenu);
        self.layers.switch(Layer::FolderName, &Layer::SIDEBAR);
        self.folder_name.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn save_folder_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.layers.contains(Layer::FolderName) {
            return;
        }
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let name = self.folder_name.read(cx).value().trim().to_string();
        // Only on success: a refused name keeps the field open.
        fn close(this: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
            this.layers.close(Layer::FolderName);
            this.folder_target = None;
            this.focus_composer(window, cx);
        }
        match self.folder_target.clone() {
            Some(folder) => {
                let reply = self.backend.rename_folder(project, folder, name);
                self.organise(
                    reply,
                    |this, (), window, cx| close(this, window, cx),
                    window,
                    cx,
                );
            }
            None => {
                let reply = self.backend.create_folder(project, name);
                self.organise(
                    reply,
                    |this, _, window, cx| {
                        close(this, window, cx);
                        // A new group should be somewhere to be seen.
                        let shown = Arrangement {
                            grouping: Grouping::Groups,
                            ..this.arrangement
                        };
                        this.set_arrangement(shown, window, cx);
                    },
                    window,
                    cx,
                );
            }
        }
    }

    fn cancel_sidebar_layer(&mut self, layer: Layer, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.close(layer) {
            if layer == Layer::FolderName {
                self.focus_composer(window, cx);
            }
            cx.notify();
        }
    }

    fn toggle_folder_menu(&mut self, folder: String, cx: &mut Context<Self>) {
        let showing =
            self.layers.contains(Layer::FolderMenu) && self.folder_target.as_ref() == Some(&folder);
        for layer in Layer::SIDEBAR {
            self.layers.close(layer);
        }
        self.folder_target = None;
        if !showing {
            self.folder_target = Some(folder);
            self.layers.open(Layer::FolderMenu);
        }
        cx.notify();
    }

    fn move_folder(&mut self, folder: &str, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let Some(order) = organise::moved(&self.folders, folder, up) else {
            return;
        };
        // Shown at once; the reload that follows confirms it.
        self.folders
            .sort_by_key(|folder| order.iter().position(|id| id == &folder.id));
        let reply = self.backend.reorder_folders(project, order);
        self.organise(reply, |_, (), _, _| {}, window, cx);
    }

    fn ask_delete_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        self.folder_target = Some(folder);
        self.layers
            .switch(Layer::ConfirmDeleteFolder, &Layer::SIDEBAR);
        cx.notify();
    }

    fn delete_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.contains(Layer::ConfirmDeleteFolder) {
            return;
        }
        let (Some(project), Some(folder)) =
            (self.state.project.clone(), self.folder_target.clone())
        else {
            return;
        };
        let key = format!("folder:{folder}");
        let reply = self.backend.delete_folder(project, folder);
        self.organise(
            reply,
            move |this, (), _, _| {
                this.layers.close(Layer::ConfirmDeleteFolder);
                this.folder_target = None;
                this.collapsed.remove(&key);
            },
            window,
            cx,
        );
    }

    /// Puts the open conversation in a group, or with `None` out of its own.
    fn group_current(
        &mut self,
        folder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::GroupMenu);
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let reply = self.backend.group_conversation(project, session, folder);
        self.organise(reply, |_, (), _, _| {}, window, cx);
    }

    /// Opens one of the conversation bar's surfaces in place of the others.
    pub(super) fn toggle_bar_layer(&mut self, layer: Layer, cx: &mut Context<Self>) {
        // A move waiting to be confirmed is not kept for the next opening.
        self.transfer_preview = None;
        if !self.layers.close(layer) {
            self.layers.switch(layer, &Layer::CONVERSATION_BAR);
        }
        cx.notify();
    }

    /// Copies or moves the open conversation to `target`, as the transfer
    /// menu is set.
    fn transfer(&mut self, target: String, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        let keep = self.transfer_copy;
        let name = self
            .projects
            .iter()
            .find(|project| project.id == target)
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "the other project".into());
        // With its artifacts: first what would go along, to confirm.
        if !keep && self.transfer_artifacts {
            let reply = self
                .backend
                .preview_move_with_artifacts(project, session, target.clone());
            self.organise(
                reply,
                move |this, preview, _, cx| {
                    this.transfer_preview = Some((target, name, preview));
                    cx.notify();
                },
                window,
                cx,
            );
            return;
        }
        self.layers.close(Layer::TransferMenu);
        let reply = self
            .backend
            .transfer_conversation(project, session.clone(), target, keep);
        self.finish_transfer(reply, session, name, keep, false, window, cx);
    }

    /// Moves the open conversation with its artifacts, as the preview read
    /// for the project chosen listed them.
    fn confirm_move_with_artifacts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(project), Some(session), Some((target, name, preview))) = (
            self.state.project.clone(),
            self.state.session.clone(),
            self.transfer_preview.take(),
        ) else {
            return;
        };
        self.layers.close(Layer::TransferMenu);
        let reply = self.backend.move_conversation_with_artifacts(
            project,
            session.clone(),
            target,
            preview.fingerprint,
        );
        self.finish_transfer(reply, session, name, false, true, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_transfer(
        &mut self,
        reply: Reply<String>,
        session: String,
        name: String,
        keep: bool,
        with_artifacts: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        self.organise(
            reply,
            move |this, _, window, cx| {
                if keep {
                    this.notice = Some(crate::i18n::fill(tr("Copied to {}."), &[&name]));
                    return;
                }
                this.known.remove(&session);
                this.state.forget_draft(&session);
                this.chips.remove(&session);
                if this.state.current(&selection) {
                    this.leave_conversation(window, cx);
                }
                this.notice = Some(if with_artifacts {
                    crate::i18n::fill(
                        tr("Moved to {} with the artifacts that could go along."),
                        &[&name],
                    )
                } else {
                    crate::i18n::fill(
                        tr("Moved to {}. Its files, runs and artifacts stay in this project."),
                        &[&name],
                    )
                });
            },
            window,
            cx,
        );
    }

    /// A line for something that went as asked.
    pub(super) fn notice_banner(&self, message: &str, cx: &mut Context<Self>) -> Div {
        let success = cx.theme().success;
        div()
            .h_flex()
            .gap_2()
            .px_3()
            .py_1()
            .rounded(cx.theme().radius)
            .bg(success.opacity(0.08))
            .text_sm()
            .child(icon("circle-check").text_color(success))
            .child(div().flex_1().min_w_0().child(message.to_owned()))
            .child(
                icon_button("notice-close", "close", tr("Dismiss"))
                    .xsmall()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.notice = None;
                        cx.notify();
                    })),
            )
    }

    fn sort_menu(&self, cx: &mut Context<Self>) -> Div {
        let mut menu = popup(190., cx)
            .top(px(34.))
            .right(px(4.))
            .child(caption(tr("Group by"), cx));
        for grouping in Grouping::ALL {
            let next = Arrangement {
                grouping,
                ..self.arrangement
            };
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("group-by-{}", grouping.label())),
                    grouping.label(),
                    "",
                    self.arrangement.grouping == grouping,
                    cx,
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.set_arrangement(next, window, cx)),
                ),
            );
        }
        menu = menu.child(caption(tr("Sort by"), cx));
        for sorting in Sorting::ALL {
            let next = Arrangement {
                sorting,
                ..self.arrangement
            };
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("sort-by-{}", sorting.label())),
                    sorting.label(),
                    "",
                    self.arrangement.sorting == sorting,
                    cx,
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.set_arrangement(next, window, cx)),
                ),
            );
        }
        menu
    }

    /// The rows of a "which group" menu. `batch` sends the choice to the
    /// picked conversations instead of the open one; `current` is checked.
    fn group_choices(
        &self,
        mut menu: Div,
        current: Option<&str>,
        batch: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let prefix = if batch { "batch-group" } else { "group" };
        let choose = |folder: Option<String>| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| {
                if batch {
                    this.run_batch(BatchAction::Group(folder.clone()), tr("Moved"), window, cx)
                } else {
                    this.group_current(folder.clone(), window, cx)
                }
            })
        };
        if self.folders.is_empty() {
            menu = menu.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("This project has no groups yet.")),
            );
        }
        for folder in &self.folders {
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("{prefix}-{}", folder.id)),
                    folder.name.clone(),
                    "",
                    current == Some(folder.id.as_str()),
                    cx,
                )
                .on_click(choose(Some(folder.id.clone()))),
            );
        }
        if !self.folders.is_empty() {
            menu = menu.child(
                Self::menu_row(
                    SharedString::from(format!("{prefix}-none")),
                    tr("No group"),
                    "",
                    !batch && current.is_none(),
                    cx,
                )
                .on_click(choose(None)),
            );
        }
        menu.child(
            Self::menu_row(
                SharedString::from(format!("{prefix}-new")),
                tr("New group…"),
                "",
                false,
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| this.begin_folder_name(None, window, cx))),
        )
    }

    /// The heading over the conversation list, with what can be done to it.
    pub(super) fn render_list_heading(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let disabled = self.pending || self.state.project.is_none();
        let heading = div()
            .relative()
            .h_flex()
            .gap_0p5()
            .px_2()
            .pt_2()
            .flex_shrink_0();
        if self.layers.contains(Layer::SelectMode) {
            let shown = organise::visible(&self.session_rows(cx), &self.sessions);
            let all = self.picks.covers(&shown);
            return heading
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::i18n::fill(tr("{} selected"), &[&self.picks.len()])),
                )
                .child(
                    Button::new("select-all")
                        .label(if all { tr("None") } else { tr("All") })
                        .xsmall()
                        .ghost()
                        .disabled(shown.is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.pick_all(cx))),
                )
                .child(
                    Button::new("select-done")
                        .label(tr("Done"))
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_select_mode(cx))),
                );
        }
        let sorting = self.layers.contains(Layer::SortMenu);
        let mut heading = heading.child(div().flex_1().text_xs().text_color(muted).child(
            if self.show_archived {
                tr("Archived")
            } else {
                tr("Conversations")
            },
        ));
        if !self.show_archived {
            heading = heading.child(
                icon_button("new-group", "folder-plus", tr("New group"))
                    .xsmall()
                    .disabled(disabled)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.begin_folder_name(None, window, cx)),
                    ),
            );
            heading = heading.child(
                icon_button(
                    "import-conversations",
                    "conversation-import",
                    "Import from Codex or Claude Code",
                )
                .xsmall()
                .disabled(disabled)
                .on_click(cx.listener(|this, _, window, cx| this.open_imports(window, cx))),
            );
        }
        heading =
            heading
                .child(
                    icon_button("select", "check-square", tr("Select conversations"))
                        .xsmall()
                        .disabled(disabled || self.sessions.is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_select_mode(cx))),
                )
                .child(
                    icon_button("sort", "adjustments", tr("Sort and group"))
                        .xsmall()
                        .selected(sorting)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_sidebar_layer(Layer::SortMenu, cx)
                        })),
                )
                .child(
                    icon_button(
                        "archive-view",
                        "archive",
                        if self.show_archived {
                            tr("Back to conversations")
                        } else {
                            tr("Archived conversations")
                        },
                    )
                    .xsmall()
                    .selected(self.show_archived)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_archived = !this.show_archived;
                        this.sessions.clear();
                        this.picks.clear();
                        this.load_sessions(window, cx);
                        cx.notify();
                    })),
                );
        if sorting {
            heading = heading.child(deferred(self.sort_menu(cx)));
        }
        heading
    }

    /// The name field of a new group or of one being renamed.
    fn folder_name_row(&self, cx: &mut Context<Self>) -> Div {
        div()
            .h_flex()
            .gap_1()
            .px_1()
            .py_1()
            .flex_shrink_0()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.folder_name).small()),
            )
            .child(
                Button::new("folder-name-save")
                    .label(if self.folder_target.is_some() {
                        tr("Save")
                    } else {
                        tr("Add")
                    })
                    .xsmall()
                    .primary()
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| this.save_folder_name(window, cx))),
            )
            .child(
                icon_button("folder-name-cancel", "close", tr("Cancel"))
                    .xsmall()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cancel_sidebar_layer(Layer::FolderName, window, cx)
                    })),
            )
    }

    /// What is showing for `folder` besides its heading, if anything.
    fn folder_layer(&self, folder: Option<&str>) -> Option<Layer> {
        if folder.is_none() || self.folder_target.as_deref() != folder {
            return None;
        }
        [
            Layer::FolderName,
            Layer::FolderMenu,
            Layer::ConfirmDeleteFolder,
        ]
        .into_iter()
        .find(|layer| self.layers.contains(*layer))
    }

    fn section_heading(
        &self,
        key: &str,
        label: &str,
        count: usize,
        folder: Option<&str>,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().sidebar_accent;
        let section = key.to_owned();
        let mut heading = div().h_flex().gap_0p5().mt_1().flex_shrink_0().child(
            div()
                .id(SharedString::from(format!("section-{key}")))
                .h_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .px_1()
                .h(px(26.))
                .rounded(cx.theme().radius)
                .cursor_pointer()
                .hover(move |style| style.bg(accent.opacity(0.5)))
                .child(
                    icon(if collapsed {
                        "chevron-right"
                    } else {
                        "chevron-down"
                    })
                    .text_color(muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(muted)
                        .child(label.to_owned()),
                )
                .child(div().text_xs().text_color(muted).child(count.to_string()))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(&section, cx))),
        );
        if let Some(folder) = folder {
            let id = folder.to_owned();
            heading = heading.child(
                icon_button(
                    SharedString::from(format!("folder-more-{folder}")),
                    "more",
                    tr("Group actions"),
                )
                .xsmall()
                .selected(self.folder_layer(Some(folder)) == Some(Layer::FolderMenu))
                .disabled(self.pending)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.toggle_folder_menu(id.clone(), cx)),
                ),
            );
        }
        heading
    }

    /// Rename, reorder and delete for one group, under its heading.
    fn folder_actions(&self, folder: &str, cx: &mut Context<Self>) -> Div {
        let first = self.folders.first().is_some_and(|other| other.id == folder);
        let last = self.folders.last().is_some_and(|other| other.id == folder);
        let (rename, up, down, delete) = (
            folder.to_owned(),
            folder.to_owned(),
            folder.to_owned(),
            folder.to_owned(),
        );
        div()
            .h_flex()
            .gap_0p5()
            .pl(px(22.))
            .pb_1()
            .flex_shrink_0()
            .child(
                Button::new("folder-rename")
                    .label(tr("Rename"))
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.begin_folder_name(Some(rename.clone()), window, cx)
                    })),
            )
            .child(
                icon_button("folder-up", "chevron-up", tr("Move up"))
                    .xsmall()
                    .disabled(first || self.pending)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.move_folder(&up, true, window, cx)
                    })),
            )
            .child(
                icon_button("folder-down", "chevron-down", tr("Move down"))
                    .xsmall()
                    .disabled(last || self.pending)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.move_folder(&down, false, window, cx)
                    })),
            )
            .child(
                Button::new("folder-delete")
                    .label(tr("Delete"))
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask_delete_folder(delete.clone(), cx)
                    })),
            )
    }

    fn folder_delete_question(&self, cx: &mut Context<Self>) -> Div {
        div()
            .v_flex()
            .gap_1()
            .pl(px(22.))
            .pr_1()
            .pb_1()
            .flex_shrink_0()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(tr("Delete this group? Its conversations are kept.")),
            )
            .child(
                div()
                    .h_flex()
                    .gap_1()
                    .child(
                        Button::new("folder-delete-confirm")
                            .label(tr("Delete group"))
                            .xsmall()
                            .danger()
                            .disabled(self.pending)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.delete_folder(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("folder-delete-cancel")
                            .label(tr("Cancel"))
                            .xsmall()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel_sidebar_layer(Layer::ConfirmDeleteFolder, window, cx)
                            })),
                    ),
            )
    }

    fn session_row(
        &self,
        index: usize,
        depth: usize,
        selecting: bool,
        indented: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(session) = self.sessions.get(index) else {
            return div().into_any_element();
        };
        let id = session.id.clone();
        let accent = cx.theme().sidebar_accent;
        let open = Some(&id) == self.state.session.as_ref();
        let picked = selecting && self.picks.contains(&id);
        let mut row = div()
            .id(SharedString::from(format!("session-{id}")))
            .h_flex()
            .gap_2()
            .px_2()
            .pl(px(8. + if indented && !selecting { 14. } else { 0. }
                // Deep chains stop stepping in before the title has no room.
                + 14. * depth.min(3) as f32))
            .h(px(30.))
            .flex_shrink_0()
            .rounded(cx.theme().radius)
            .text_sm()
            .when(open && !selecting, |row| row.bg(accent))
            .when(picked, |row| row.bg(accent.opacity(0.6)));
        if selecting {
            row = row.child(if picked {
                icon("check-square").text_color(cx.theme().primary)
            } else {
                icon("square").text_color(cx.theme().muted_foreground)
            });
        }
        if depth > 0 {
            row = row.child(icon("branch").text_color(cx.theme().muted_foreground));
        }
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(crate::present::one_line(&session.title)),
        );
        if let Some((label, tone)) = present::session_badge(session, self.pinned.contains(&id)) {
            row = row.child(chip(label, tone, cx));
        }
        if !self.pending {
            row = row
                .cursor_pointer()
                .hover(move |style| style.bg(accent.opacity(0.7)))
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if this.layers.contains(Layer::SelectMode) {
                        this.pick(&id, event.modifiers().shift, cx)
                    } else {
                        this.select_session(id.clone(), window, cx)
                    }
                }));
        }
        row.into_any_element()
    }

    /// The conversations of the project, under their sections.
    pub(super) fn render_session_list(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let rows = self.session_rows(cx);
        let selecting = self.layers.contains(Layer::SelectMode);
        let sectioned = rows.iter().any(|row| matches!(row, Row::Section { .. }));
        let mut list = div()
            .id("sessions")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        // A new group has no heading yet; its name field leads the list.
        if self.layers.contains(Layer::FolderName) && self.folder_target.is_none() {
            list = list.child(self.folder_name_row(cx));
        }
        // The assistant's project is not listed: its one conversation is
        // the one open.
        if crate::assistant::is_assistant_project(self.state.project.as_deref()) {
            return list.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("The assistant keeps one conversation, the one open.")),
            );
        }
        if rows.is_empty() && self.state.project.is_some() {
            list = list.child(div().px_2().py_1().text_xs().text_color(muted).child(
                match self.listing(cx) {
                    Listing::Search => tr("No conversation matches."),
                    Listing::Archive => tr("Nothing is archived."),
                    Listing::Browse => tr("No conversations yet."),
                },
            ));
        }
        for row in rows {
            match row {
                Row::Conversation { index, depth } => {
                    list = list.child(self.session_row(index, depth, selecting, sectioned, cx));
                }
                Row::Section {
                    key,
                    label,
                    count,
                    folder,
                    collapsed,
                } => {
                    let layer = self.folder_layer(folder.as_deref());
                    if layer == Some(Layer::FolderName) {
                        list = list.child(self.folder_name_row(cx));
                        continue;
                    }
                    list = list.child(self.section_heading(
                        &key,
                        &label,
                        count,
                        folder.as_deref(),
                        collapsed,
                        cx,
                    ));
                    match (layer, folder.as_deref()) {
                        (Some(Layer::FolderMenu), Some(folder)) => {
                            list = list.child(self.folder_actions(folder, cx));
                        }
                        (Some(Layer::ConfirmDeleteFolder), Some(_)) => {
                            list = list.child(self.folder_delete_question(cx));
                        }
                        _ => {}
                    }
                }
            }
        }
        list
    }

    /// What can be done to the picked conversations, under the list.
    pub(super) fn render_batch_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.layers.contains(Layer::SelectMode) {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let count = self.picks.len();
        let idle = self.picks.is_empty() || self.pending;
        let bar = div()
            .v_flex()
            .gap_1()
            .pt_1()
            .flex_shrink_0()
            .border_t_1()
            .border_color(cx.theme().sidebar_border);
        if self.layers.contains(Layer::ConfirmBatchDelete) {
            let what = if count == 1 {
                tr("Delete 1 conversation and its messages permanently? \
                 Project files it wrote are kept.")
                .to_string()
            } else {
                crate::i18n::fill(
                    tr("Delete {} conversations and their messages permanently? \
                     Project files they wrote are kept."),
                    &[&count],
                )
            };
            return Some(
                bar.child(
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(what),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_1()
                        .px_1()
                        .child(
                            Button::new("batch-delete-confirm")
                                .label(tr("Delete permanently"))
                                .xsmall()
                                .danger()
                                .disabled(idle)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.run_batch(BatchAction::Delete, tr("Deleted"), window, cx)
                                })),
                        )
                        .child(
                            Button::new("batch-delete-cancel")
                                .label(tr("Cancel"))
                                .xsmall()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.cancel_sidebar_layer(Layer::ConfirmBatchDelete, window, cx)
                                })),
                        ),
                ),
            );
        }
        let archived = self.show_archived;
        let grouping = self.layers.contains(Layer::BatchGroupMenu);
        let mut group = div().relative().child(
            Button::new("batch-group")
                .icon(glyph("folder-move"))
                .label(tr("Group"))
                .xsmall()
                .ghost()
                .selected(grouping)
                .disabled(idle)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toggle_sidebar_layer(Layer::BatchGroupMenu, cx)
                })),
        );
        if grouping {
            let menu = popup(220., cx).bottom(px(28.)).left_0();
            group = group.child(deferred(self.group_choices(menu, None, true, cx)));
        }
        let mut bar = bar.child(
            div()
                .h_flex()
                .gap_0p5()
                .px_1()
                .child(group)
                .child(
                    Button::new("batch-archive")
                        .icon(glyph("archive"))
                        .label(if archived {
                            tr("Restore")
                        } else {
                            tr("Archive")
                        })
                        .xsmall()
                        .ghost()
                        .disabled(idle)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if archived {
                                this.run_batch(BatchAction::Restore, tr("Restored"), window, cx)
                            } else {
                                this.run_batch(BatchAction::Archive, tr("Archived"), window, cx)
                            }
                        })),
                )
                .child(
                    Button::new("batch-delete")
                        .icon(glyph("trash"))
                        .label(tr("Delete"))
                        .xsmall()
                        .ghost()
                        .disabled(idle)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_sidebar_layer(Layer::ConfirmBatchDelete, cx)
                        })),
                ),
        );
        if count == 0 {
            bar = bar.child(div().px_2().text_xs().text_color(muted).child(tr(
                "Click conversations to pick them. Shift-click picks a range.",
            )));
        }
        Some(bar)
    }

    /// The Group button of the conversation bar, with its menu when open.
    pub(super) fn group_button(&self, disabled: bool, cx: &mut Context<Self>) -> Div {
        let open = self.layers.contains(Layer::GroupMenu);
        let mut button = div().relative().child(
            Button::new("conversation-group")
                .label(tr("Group"))
                .small()
                .ghost()
                .selected(open)
                .disabled(disabled)
                .on_click(
                    cx.listener(|this, _, _, cx| this.toggle_bar_layer(Layer::GroupMenu, cx)),
                ),
        );
        if open {
            let current = self
                .state
                .session
                .as_ref()
                .and_then(|id| self.grouped.get(id))
                .map(String::as_str);
            let menu = popup(220., cx).top(px(32.)).left_0();
            button = button.child(deferred(self.group_choices(menu, current, false, cx)));
        }
        button
    }

    /// The Move button of the conversation bar, with the projects the
    /// conversation can go to when open.
    pub(super) fn transfer_button(&self, disabled: bool, cx: &mut Context<Self>) -> Div {
        let open = self.layers.contains(Layer::TransferMenu);
        let mut button = div().relative().child(
            Button::new("conversation-transfer")
                .label(tr("Move"))
                .small()
                .ghost()
                .selected(open)
                .disabled(disabled)
                .on_click(
                    cx.listener(|this, _, _, cx| this.toggle_bar_layer(Layer::TransferMenu, cx)),
                ),
        );
        if !open {
            return button;
        }
        let muted = cx.theme().muted_foreground;
        let copy = self.transfer_copy;
        let mode = |id: &'static str, label: &'static str, value: bool| {
            Button::new(id)
                .label(label)
                .xsmall()
                .ghost()
                .selected(copy == value)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.transfer_copy = value;
                    cx.notify();
                }))
        };
        let mut menu = popup(300., cx).top(px(32.)).left_0().child(
            div()
                .h_flex()
                .gap_1()
                .px_1()
                .child(mode("transfer-move", tr("Move"), false))
                .child(mode("transfer-copy", tr("Copy"), true)),
        );
        // A move waiting for its artifacts' preview to be confirmed.
        if let Some((_, name, preview)) = &self.transfer_preview {
            let mut confirm = div().v_flex().gap_1().px_2().py_1().text_xs().child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(crate::i18n::fill(tr("Move to {}"), &[name])),
            );
            for line in crate::organise::move_preview_lines(preview) {
                confirm = confirm.child(div().text_color(muted).child(line));
            }
            confirm = confirm.child(
                div()
                    .h_flex()
                    .gap_1()
                    .pt_1()
                    .child(
                        Button::new("transfer-confirm")
                            .label(tr("Move"))
                            .xsmall()
                            .primary()
                            .disabled(self.pending)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_move_with_artifacts(window, cx)
                            })),
                    )
                    .child(
                        Button::new("transfer-back")
                            .label(tr("Back"))
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.transfer_preview = None;
                                cx.notify();
                            })),
                    ),
            );
            button = button.child(deferred(menu.child(confirm)));
            return button;
        }
        menu = menu.child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(muted)
                .child(if copy {
                    tr(
                        "A copy of the transcript is added to the project you choose. \
                     This conversation stays here.",
                    )
                } else if self.transfer_artifacts {
                    tr("The transcript moves to the project you choose, with the \
                     artifacts it made and their files, unless another conversation \
                     uses them or they were changed since. Runs stay here.")
                } else {
                    tr(
                        "The transcript moves to the project you choose. Its files, \
                     runs and artifacts stay in this project.",
                    )
                }),
        );
        if !copy {
            let along = self.transfer_artifacts;
            menu = menu.child(
                div().px_1().child(
                    Button::new("transfer-artifacts")
                        .label(tr("Take its artifacts along"))
                        .xsmall()
                        .ghost()
                        .selected(along)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.transfer_artifacts = !along;
                            cx.notify();
                        })),
                ),
            );
        }
        let home_dir = dirs::home_dir().map(|path| path.to_string_lossy().into_owned());
        let mut targets = div()
            .id("transfer-targets")
            .v_flex()
            .max_h(px(260.))
            .overflow_y_scroll();
        let mut any = false;
        for project in &self.projects {
            if Some(&project.id) == self.state.project.as_ref() {
                continue;
            }
            any = true;
            let target = project.id.clone();
            targets =
                targets.child(
                    Self::menu_row(
                        SharedString::from(format!("transfer-{target}")),
                        project.name.clone(),
                        home::short_path(&project.workspace_dir, home_dir.as_deref()),
                        false,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.transfer(target.clone(), window, cx)
                    })),
                );
        }
        if !any {
            targets = targets.child(div().px_2().py_1().text_sm().text_color(muted).child(tr(
                "There is no other project yet. Open one from the home page.",
            )));
        }
        button = button.child(deferred(menu.child(targets)));
        button
    }
}
