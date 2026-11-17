//! Folders on an SSH host, in the Files tab: a look around a saved host
//! that has passed a check. A file there can be looked at, as the
//! WebView's preview reads it (text as its first megabyte, a picture whole
//! up to 32 MB), copied to this computer, and files of this computer sent
//! into the folder shown; each copy goes on as a run, with its progress,
//! and nothing there is replaced. Looking keeps nothing.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Disableable, Sizable, StyledExt, text::TextView};

use super::{
    Workspace,
    favorites_view::image_format,
    shell::{glyph, icon_button},
    surfaces,
};
use crate::{
    backend::receive,
    files::{self, RemoteLook, RemoteShown},
    layers::Layer,
};

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

    /// Shows `path` of `alias` in place of its folder's listing, read as
    /// the WebView's preview reads it. `listed` is the size the listing
    /// gave; a picture larger than a preview fetches is not fetched.
    fn look_at_remote(
        &mut self,
        alias: String,
        path: String,
        listed: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(browse) = self.remote.as_mut().filter(|browse| browse.alias == alias) else {
            return;
        };
        browse.look = Some(RemoteLook::new(alias.clone(), path.clone(), listed));
        self.remote_image = None;
        self.reset_picture(false);
        self.file_as_text = false;
        self.layers.open(Layer::RemotePreview);
        let reply = self
            .backend
            .remote_file(alias.clone(), path.clone(), listed);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Another file, another folder or the project by now.
                let Some(look) = this
                    .remote
                    .as_mut()
                    .and_then(|browse| browse.look.as_mut())
                    .filter(|look| look.alias == alias && look.path == path)
                else {
                    return;
                };
                match result {
                    Ok(file) => {
                        look.answered(&file);
                        match file.body {
                            wisp_server::desktop::RemoteFileBody::Text { text, .. } => this
                                .remote_text
                                .update(cx, |input, cx| input.set_value(text, window, cx)),
                            wisp_server::desktop::RemoteFileBody::Picture {
                                content_type,
                                bytes,
                            } => match image_format(Some(&content_type)) {
                                Some(format) => {
                                    this.remote_image = Some((
                                        remote_key(&alias, &path),
                                        std::sync::Arc::new(Image::from_bytes(format, bytes)),
                                    ))
                                }
                                None => {
                                    look.shown = Some(Err(tr(
                                        "This kind of picture cannot be shown here.",
                                    )
                                    .into()))
                                }
                            },
                            _ => {}
                        }
                    }
                    Err(error) => look.shown = Some(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Back from a host's file to its folder. Its bytes go with it.
    pub(super) fn close_remote_look(&mut self) {
        if let Some(browse) = self.remote.as_mut() {
            browse.look = None;
        }
        self.remote_image = None;
        self.layers.close(Layer::RemotePreview);
    }

    /// Back from a host to the project's own files.
    pub(super) fn leave_host(&mut self) {
        self.close_remote_look();
        self.remote = None;
    }

    /// Lists `path` on `alias`; an empty path is the home folder.
    pub(super) fn browse_host(
        &mut self,
        alias: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_remote_look();
        self.remote = Some(files::RemoteBrowse {
            alias: alias.clone(),
            path: path.clone(),
            listing: None,
            error: None,
            look: None,
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
        // One choice among the places: a segmented control, this project
        // (`computer`) and each host (`server`).
        let mut places = vec![
            surfaces::segment(
                "location-project",
                "computer",
                tr("This project"),
                on_host.is_none(),
                cx,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.leave_host();
                cx.notify();
            }))
            .into_any_element(),
        ];
        for host in hosts {
            let alias = host.alias.clone();
            places.push(
                surfaces::segment(
                    SharedString::from(format!("location-{alias}")),
                    "server",
                    alias.clone(),
                    on_host == Some(alias.as_str()),
                    cx,
                )
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new(tr("Look at this host's folders"))
                        .build(window, cx)
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.browse_host(alias.clone(), String::new(), window, cx)
                }))
                .into_any_element(),
            );
        }
        let row = div()
            .h_flex()
            .px_3()
            .pt_2()
            .overflow_x_hidden()
            .child(surfaces::segmented(places, cx));
        Some(row)
    }

    /// A file of a host, in place of its folder: its name and size, what
    /// can be shown of it with the project preview's own renderers, and
    /// why not when nothing can.
    fn render_remote_look(
        &self,
        look: &RemoteLook,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let name = files::file_name(&look.path).to_owned();
        let note = look.note();
        let (from, path, saved_as) = (look.alias.clone(), look.path.clone(), name.clone());
        let header = surfaces::panel_header(name, (!note.is_empty()).then(|| note.into()), cx)
            .child(
                icon_button(
                    "remote-look-download",
                    "download",
                    "Copy it to this computer…",
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.download_remote(from.clone(), path.clone(), saved_as.clone(), window, cx)
                })),
            )
            .child(
                icon_button("remote-look-back", "arrow-left", "Back to files").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.close_remote_look();
                        cx.notify();
                    }),
                ),
            );
        let body = match (&look.shown, look.message()) {
            (None, _) => surfaces::panel_empty(tr("Loading preview…"), cx).into_any_element(),
            (Some(Err(_)), Some(message)) => surfaces::panel_empty(crate::i18n::host(&message), cx)
                .text_color(cx.theme().danger)
                .into_any_element(),
            (_, Some(message)) => surfaces::panel_empty(message, cx).into_any_element(),
            (Some(Ok(RemoteShown::Picture)), None) => {
                let key = remote_key(&look.alias, &look.path);
                match self
                    .remote_image
                    .as_ref()
                    .filter(|(shown, _)| *shown == key)
                {
                    Some((key, image)) => self.render_picture(key, image.clone(), window, cx),
                    None => {
                        surfaces::panel_empty(tr("Loading the picture…"), cx).into_any_element()
                    }
                }
            }
            (Some(Ok(RemoteShown::Text { text, .. })), None) => {
                if files::shows_as_markdown(&look.path, text) {
                    div()
                        .flex_1()
                        .min_h_0()
                        .p_3()
                        .child(
                            TextView::markdown(
                                SharedString::from(format!(
                                    "remote-preview-{}",
                                    remote_key(&look.alias, &look.path)
                                )),
                                text.clone(),
                                window,
                                cx,
                            )
                            .selectable(true)
                            .scrollable(true),
                        )
                        .into_any_element()
                } else if let Some(table) = files::table(&look.path, text) {
                    self.render_table(&table, &self.remote_text, cx)
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(
                            super::surfaces::input(&self.remote_text, cx)
                                .disabled(true)
                                .h_full(),
                        )
                        .into_any_element()
                }
            }
            // Every other answer has a message.
            (Some(_), None) => div().into_any_element(),
        };
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .child(header)
            .child(
                div()
                    .px_3()
                    .pb_1()
                    .text_xs()
                    .text_color(muted)
                    .child(crate::i18n::fill(
                        tr("On {}: {}"),
                        &[&look.alias, &look.path],
                    )),
            )
            .child(body)
    }

    /// The chosen host's folder, when one is being looked at.
    pub(super) fn render_remote_files(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let browse = self.remote.as_ref()?;
        if let Some(look) = browse
            .look
            .as_ref()
            .filter(|_| self.layers.contains(Layer::RemotePreview))
        {
            return Some(self.render_remote_look(look, window, cx));
        }
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
                                    // The row underneath shows the file.
                                    cx.stop_propagation();
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
                    let (alias, into, size) = (
                        alias.clone(),
                        files::remote_child(&listing.path, &entry.name),
                        entry.size,
                    );
                    let is_dir = entry.is_dir;
                    row = row
                        .cursor_pointer()
                        .hover(|row| row.bg(cx.theme().list_hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if is_dir {
                                this.browse_host(alias.clone(), into.clone(), window, cx)
                            } else {
                                this.look_at_remote(
                                    alias.clone(),
                                    into.clone(),
                                    Some(size),
                                    window,
                                    cx,
                                )
                            }
                        }));
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

/// What a host's file is kept under while it is shown: its host and path,
/// which no project path can be mistaken for.
fn remote_key(alias: &str, path: &str) -> String {
    format!("ssh:{alias}:{path}")
}
