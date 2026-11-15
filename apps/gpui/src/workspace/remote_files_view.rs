//! Folders on an SSH host, in the Files tab: a look around a saved host
//! that has passed a check. It only lists; nothing on the host is opened,
//! changed or copied from here.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Disableable, Selectable, Sizable, StyledExt, button::Button};

use super::{Workspace, shell::glyph};
use crate::{backend::receive, files};

impl Workspace {
    /// Lists `path` on `alias`; an empty path is the home folder.
    pub(super) fn browse_host(
        &mut self,
        alias: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remote = Some(files::RemoteBrowse {
            alias: alias.clone(),
            path: path.clone(),
            listing: None,
            error: None,
        });
        let reply = self.backend.remote_directory(alias.clone(), path.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                // Another folder, or the project again, was chosen meanwhile.
                let Some(browse) = this
                    .remote
                    .as_mut()
                    .filter(|browse| browse.alias == alias && browse.path == path)
                else {
                    return;
                };
                match result {
                    Ok(listing) => browse.listing = Some(listing),
                    Err(error) => browse.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The places the Files tab can show: the project, and each saved host.
    /// Nothing when there is no host to go to.
    pub(super) fn render_locations(&self, cx: &mut Context<Self>) -> Option<Div> {
        let hosts = &self.environments.as_ref()?.hosts;
        if hosts.is_empty() {
            return None;
        }
        let on_host = self.remote.as_ref().map(|browse| browse.alias.as_str());
        let mut row = div().h_flex().flex_wrap().gap_1().px_3().pt_2().child(
            Button::new("location-project")
                .label(tr("This project"))
                .xsmall()
                .selected(on_host.is_none())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.remote = None;
                    cx.notify();
                })),
        );
        for host in hosts {
            let alias = host.alias.clone();
            row = row.child(
                Button::new(SharedString::from(format!("location-{alias}")))
                    .icon(glyph("server"))
                    .label(alias.clone())
                    .xsmall()
                    .selected(on_host == Some(alias.as_str()))
                    .tooltip(tr("Look at this host's folders"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.browse_host(alias.clone(), String::new(), window, cx)
                    })),
            );
        }
        Some(row)
    }

    /// The chosen host's folder, when one is being looked at.
    pub(super) fn render_remote_files(&self, cx: &mut Context<Self>) -> Option<Div> {
        let browse = self.remote.as_ref()?;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let shown = browse
            .listing
            .as_ref()
            .map_or(browse.path.as_str(), |listing| listing.path.as_str());
        let parent = files::remote_parent(shown);
        let alias = browse.alias.clone();
        let (up_alias, again_alias, again_path) =
            (alias.clone(), alias.clone(), browse.path.clone());
        let mut view = div().v_flex().flex_1().min_h_0().child(
            div()
                .v_flex()
                .gap_2()
                .p_3()
                .border_b_1()
                .border_color(border)
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .child(match shown {
                            "" => tr("Home folder").to_owned(),
                            path => path.to_owned(),
                        }),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_1()
                        .child(
                            Button::new("remote-up")
                                .label(tr("Up"))
                                .small()
                                .disabled(parent.is_none())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(parent) = parent.clone() {
                                        this.browse_host(up_alias.clone(), parent, window, cx);
                                    }
                                })),
                        )
                        .child({
                            let (context, folder) = (format!("ssh:{alias}"), shown.to_owned());
                            Button::new("remote-terminal")
                                .label(tr("Terminal"))
                                .small()
                                .tooltip(tr("Open a terminal on this host, in this folder"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_host_terminal(
                                        context.clone(),
                                        Some(folder.clone()),
                                        window,
                                        cx,
                                    )
                                }))
                        })
                        .child(
                            Button::new("remote-reload")
                                .label(tr("Reload"))
                                .small()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.browse_host(
                                        again_alias.clone(),
                                        again_path.clone(),
                                        window,
                                        cx,
                                    )
                                })),
                        ),
                ),
        );
        let note = |text: String, color: Hsla| div().p_3().text_sm().text_color(color).child(text);
        let mut list = div()
            .id("remote-files")
            .v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        match (&browse.listing, &browse.error) {
            (_, Some(error)) => list = list.child(note(error.clone(), cx.theme().danger)),
            (None, None) => list = list.child(note(tr("Loading…").into(), muted)),
            (Some(listing), None) => {
                if listing.entries.is_empty() {
                    list = list.child(note(tr("This folder is empty.").into(), muted));
                }
                for (index, entry) in listing.entries.iter().enumerate() {
                    let mut row = div()
                        .id(("remote-entry", index))
                        .h_flex()
                        .gap_2()
                        .px_3()
                        .h(px(30.))
                        .flex_shrink_0()
                        .text_sm()
                        .child(
                            glyph(if entry.is_dir { "folder" } else { "doc" })
                                .small()
                                .text_color(muted),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(entry.name.clone()),
                        )
                        .child(div().flex_shrink_0().text_xs().text_color(muted).child(
                            if entry.is_dir {
                                tr("Folder").to_owned()
                            } else {
                                files::size_label(entry.size)
                            },
                        ));
                    if entry.is_dir {
                        let (alias, into) = (
                            alias.clone(),
                            files::remote_child(&listing.path, &entry.name),
                        );
                        row = row
                            .cursor_pointer()
                            .hover(|row| row.bg(cx.theme().list_hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.browse_host(alias.clone(), into.clone(), window, cx)
                            }));
                    }
                    list = list.child(row);
                }
                if listing.cut {
                    list = list.child(note(
                        tr("There is more in this folder than is listed.").into(),
                        muted,
                    ));
                }
            }
        }
        view = view.child(list);
        Some(view)
    }
}
