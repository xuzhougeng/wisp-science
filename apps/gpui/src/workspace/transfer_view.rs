//! Moving a whole project between computers: writing it as an archive and
//! reading one back in. Where the archive goes, which one is read and
//! where the project is put are all chosen in the system's own dialogs.
use crate::i18n::tr;
use gpui::{prelude::*, *};

use super::Workspace;
use crate::{backend::receive, organise};

impl Workspace {
    /// Asks where to save and writes the project there as one zip.
    pub(super) fn export_project(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        let name = self
            .projects
            .iter()
            .find(|row| row.id == project)
            .map(|row| row.name.clone())
            .unwrap_or_default();
        self.error = None;
        self.notice = None;
        let folder = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let place = cx.prompt_for_new_path(&folder, Some(&organise::archive_name(&name)));
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = place.await else {
                // The dialog was dismissed: nothing is written.
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.pending = true;
                this.notice = Some(crate::i18n::fill(tr("Writing {}…"), &[&path.display()]));
                cx.notify();
            });
            let result = receive(backend.export_project(project, path.clone())).await;
            let _ = this.update(cx, |this, cx| {
                this.pending = false;
                match result {
                    Ok(()) => {
                        this.notice = Some(crate::i18n::fill(tr("Saved to {}"), &[&path.display()]))
                    }
                    Err(error) => {
                        this.notice = None;
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Asks for an archive, then for the folder to put the project in, and
    /// reads it in. Nothing is changed until both are chosen.
    pub(super) fn import_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.ready {
            return;
        }
        self.error = None;
        self.notice = None;
        let archive = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Import").into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(archives))) = archive.await else {
                return;
            };
            let Some(archive) = archives.into_iter().next() else {
                return;
            };
            let Ok(parent) = cx.update(|_, cx| {
                cx.prompt_for_paths(PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: Some(tr("Put the project here").into()),
                })
            }) else {
                return;
            };
            let Ok(Ok(Some(parents))) = parent.await else {
                return;
            };
            let Some(parent) = parents.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.pending = true;
                this.notice = Some(crate::i18n::fill(tr("Reading {}…"), &[&archive.display()]));
                cx.notify();
            });
            let result = receive(backend.import_project(archive, parent)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(_) => {
                        this.notice = Some(tr("The project was imported.").into());
                        this.load_projects(window, cx);
                        this.load_recent(window, cx);
                    }
                    Err(error) => {
                        this.notice = None;
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Asks where to put a sample project, makes it there and opens it.
    /// Nothing is written until a folder is chosen.
    pub(super) fn create_sample_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.ready {
            return;
        }
        self.error = None;
        self.notice = None;
        let parent = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Put the sample here").into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(parents))) = parent.await else {
                return;
            };
            let Some(parent) = parents.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.pending = true;
                cx.notify();
            });
            let result = receive(backend.create_sample_project(parent)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(project) => {
                        this.notice = Some(
                            tr("The sample project is ready. Its README says what to try.").into(),
                        );
                        this.load_projects(window, cx);
                        this.enter_project(project, window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
