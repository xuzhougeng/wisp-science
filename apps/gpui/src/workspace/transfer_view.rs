//! Moving a whole project between computers: writing it as an archive or
//! as a folder package, and reading either back in. Where it goes, which
//! one is read and where an archive's project is put are all chosen in the
//! system's own dialogs; a folder package is opened where it is.
use crate::i18n::tr;
use gpui::{prelude::*, *};

use super::Workspace;
use crate::{backend::receive, organise};

impl Workspace {
    /// Says how far the export or import under way has got, read from the
    /// host every moment until it ends.
    fn follow_archive(&mut self, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(300))
                    .await;
                let progress = receive(backend.archive_progress()).await.ok().flatten();
                let going = this.update(cx, |this, cx| {
                    if !this.pending {
                        return false;
                    }
                    if let Some(progress) = &progress {
                        this.notice = Some(organise::archive_line(progress));
                        cx.notify();
                    }
                    true
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

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
                this.follow_archive(cx);
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

    /// Asks where to put a folder package of the project, and writes it
    /// there in a new folder named after it.
    pub(super) fn export_project_folder(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        self.error = None;
        self.notice = None;
        let parent = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Put the package here").into()),
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
                this.follow_archive(cx);
                this.notice = Some(crate::i18n::fill(tr("Writing {}…"), &[&parent.display()]));
                cx.notify();
            });
            let result = receive(backend.export_project_folder(project, parent)).await;
            let _ = this.update(cx, |this, cx| {
                this.pending = false;
                match result {
                    Ok(package) => {
                        this.notice =
                            Some(crate::i18n::fill(tr("Saved to {}"), &[&package.display()]))
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

    /// Asks for an archive or a folder package. A folder package is opened
    /// where it is; for an archive, the folder to put the project in is
    /// asked next. Nothing is changed until all is chosen.
    pub(super) fn import_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.ready {
            return;
        }
        self.error = None;
        self.notice = None;
        let archive = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
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
            if archive.is_dir() {
                let _ = this.update(cx, |this, cx| {
                    this.pending = true;
                    this.follow_archive(cx);
                    this.notice = Some(crate::i18n::fill(tr("Reading {}…"), &[&archive.display()]));
                    cx.notify();
                });
                let result = receive(backend.import_project_folder(archive)).await;
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
                return;
            }
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
                this.follow_archive(cx);
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
