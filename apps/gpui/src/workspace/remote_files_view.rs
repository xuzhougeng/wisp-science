//! Folders on an SSH host, in the Files tab: a look around a saved host
//! that has passed a check. A file there can be copied to this computer,
//! and files of this computer sent into the folder shown; each copy goes
//! on as a run, with its progress, and nothing there is replaced.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Disableable, Selectable, Sizable, StyledExt, button::Button};

use super::{
    Workspace,
    shell::{glyph, icon_button},
    surfaces,
};
use crate::{backend::receive, files};

impl Workspace {
    /// Asks where on this computer a host's file goes, then copies it there.
    fn download_remote(
        &mut self,
        alias: String,
        path: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        let folder = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let place = cx.prompt_for_new_path(&folder, Some(&name));
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let Some(destination) = place
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?
                else {
                    return Ok(None);
                };
                receive(backend.download_from_host(
                    project,
                    selection.session.clone(),
                    alias,
                    path,
                    destination.clone(),
                ))
                .await?;
                Ok::<_, String>(Some(destination))
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Some(destination)) => {
                        this.notice = Some(crate::i18n::fill(
                            tr("Copying to {}; the Runs tab shows how far it has got."),
                            &[&destination.display()],
                        ));
                        this.start_poll(window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks for files of this computer and sends them into the folder shown.
    fn upload_remote(
        &mut self,
        alias: String,
        folder: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(tr("Send").into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let picked = picker
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?;
                match picked.filter(|paths| !paths.is_empty()) {
                    Some(paths) => receive(
                        backend.upload_to_host(
                            project,
                            selection.session.clone(),
                            alias,
                            folder,
                            paths
                                .iter()
                                .map(|path| path.to_string_lossy().into_owned())
                                .collect(),
                        ),
                    )
                    .await
                    .map(|runs| Some(runs.len())),
                    None => Ok(None),
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Some(count)) => {
                        this.notice = Some(crate::i18n::fill(
                            tr("Sending {} file(s); reload the folder once they are there."),
                            &[&count],
                        ));
                        this.start_poll(window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

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
        let shown = browse
            .listing
            .as_ref()
            .map_or(browse.path.as_str(), |listing| listing.path.as_str());
        let parent = files::remote_parent(shown);
        let alias = browse.alias.clone();
        let (up_alias, again_alias, again_path) =
            (alias.clone(), alias.clone(), browse.path.clone());
        let place = match shown {
            "" => tr("Home folder").to_owned(),
            path => path.to_owned(),
        };
        let mut view = div().v_flex().flex_1().min_h_0().child(
            surfaces::panel_header(place, None, cx)
                .child(
                    icon_button("remote-up", "up", "Up")
                        .disabled(parent.is_none())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(parent) = parent.clone() {
                                this.browse_host(up_alias.clone(), parent, window, cx);
                            }
                        })),
                )
                .child({
                    let (to, folder) = (alias.clone(), shown.to_owned());
                    icon_button(
                        "remote-send",
                        "upload",
                        "Copy files of this computer into this folder",
                    )
                    .disabled(browse.listing.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.upload_remote(to.clone(), folder.clone(), window, cx)
                    }))
                })
                .child({
                    let (context, folder) = (format!("ssh:{alias}"), shown.to_owned());
                    icon_button(
                        "remote-terminal",
                        "terminal",
                        "Open a terminal on this host, in this folder",
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_host_terminal(context.clone(), Some(folder.clone()), window, cx)
                    }))
                })
                .child(
                    icon_button("remote-reload", "refresh", "Reload").on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.browse_host(again_alias.clone(), again_path.clone(), window, cx)
                        },
                    )),
                ),
        );
        let note = |text: String, color: Hsla| surfaces::panel_empty(text, cx).text_color(color);
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
                    let mut row = surfaces::panel_row(("remote-entry", index), cx)
                        .child(
                            glyph(if entry.is_dir { "folder" } else { "doc" })
                                .small()
                                .text_color(muted),
                        )
                        .child(surfaces::panel_label(entry.name.clone()))
                        .child(div().flex_shrink_0().text_xs().text_color(muted).child(
                            if entry.is_dir {
                                tr("Folder").to_owned()
                            } else {
                                files::size_label(entry.size)
                            },
                        ));
                    if !entry.is_dir {
                        let (from, path, name) = (
                            alias.clone(),
                            files::remote_child(&listing.path, &entry.name),
                            entry.name.clone(),
                        );
                        row = row.child(
                            icon_button(
                                ("remote-download", index),
                                "download",
                                tr("Copy it to this computer…"),
                            )
                            .xsmall()
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.download_remote(
                                        from.clone(),
                                        path.clone(),
                                        name.clone(),
                                        window,
                                        cx,
                                    )
                                },
                            )),
                        );
                    }
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
