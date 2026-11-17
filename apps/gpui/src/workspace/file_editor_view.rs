//! Editing an R or Python script where the Files tab previews it, or where
//! the centre shows it (the same preview, `centre_view`), a port of the
//! WebView's source editor (`RpCodeEditor`): the text is edited in
//! place with line numbers, saved by Save or Cmd/Ctrl+S over the file it
//! was loaded from (and refused if the file changed on disk since), and run
//! in the open conversation's interpreter for its language, a statement or
//! the selection at a time (Cmd/Ctrl+Return) or the saved script whole
//! (Cmd/Ctrl+Shift+Return), with the output in the console above the
//! composer. Unsaved edits stay per file while the window is open.
use crate::i18n::tr;
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::Position,
};

use super::{
    Workspace,
    shell::glyph,
    surfaces::{self, DialogWidth},
};
use crate::{
    backend::receive,
    files::{self, Editing, Reopened, Script},
    layers::Layer,
};

actions!(file_editor, [SaveFile, RunCode, RunScript]);

/// The key context around the editor, so its keys outrank the input's own
/// (Cmd/Ctrl+Return would otherwise start a new line) and only there. The
/// Favorites page's code editor takes it too, for Cmd/Ctrl+S.
pub(super) const CONTEXT: &str = "FileEditor";

/// The editor's keys, bound after the input component's so they win.
/// `secondary` is Cmd on macOS and Ctrl elsewhere.
pub fn file_editor_bindings() -> Vec<KeyBinding> {
    let inside = format!("{CONTEXT} > Input");
    vec![
        KeyBinding::new("secondary-s", SaveFile, Some(&inside)),
        KeyBinding::new("secondary-enter", RunCode, Some(&inside)),
        KeyBinding::new("secondary-shift-enter", RunScript, Some(&inside)),
    ]
}

/// What is run in the interpreter.
enum EditorRun {
    /// A selection or a statement, as code.
    Code(String),
    /// The saved script at this project path, whole.
    Script(String),
}

/// What follows a save that wrote the file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AfterSave {
    Nothing,
    RunScript,
}

impl Workspace {
    /// The edit of the previewed script, while that is the file shown, in
    /// the Files tab or in the centre: there is one preview, so one editor
    /// and one draft per file wherever it is drawn.
    pub(super) fn live_edit(&self) -> Option<&Editing> {
        let edit = self.file_edit.as_ref()?;
        ((self.layers.contains(Layer::FilePreview) || self.preview_in_centre())
            && self.files.preview_path.as_deref() == Some(edit.path.as_str())
            && self.state.project.as_deref() == Some(edit.project.as_str()))
        .then_some(edit)
    }

    /// Puts a script just loaded into the editor: its unsaved draft when
    /// it has one, as the WebView keeps them, otherwise the file.
    pub(super) fn begin_editing(
        &mut self,
        script: Script,
        path: String,
        disk: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let draft = self.file_drafts.get(&project, &path).cloned();
        let changed_on_disk = files::reopen(draft.as_ref(), &disk) == Reopened::ChangedOnDisk;
        let (base, text) = match draft {
            Some(draft) => (draft.base, draft.text),
            None => (disk.clone(), files::editor_text(&disk)),
        };
        self.file_edit = Some(Editing {
            project,
            path,
            script,
            dirty: files::is_dirty(&base, &text),
            base,
            saving: false,
            changed_on_disk,
            problem: None,
        });
        self.file_view
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// Keeps what the editor holds as the file's draft, after each change.
    pub(super) fn note_file_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.live_edit() else {
            return;
        };
        let (project, path, base) = (edit.project.clone(), edit.path.clone(), edit.base.clone());
        let text = self.file_view.read(cx).value().to_string();
        self.file_drafts.note(&project, &path, &base, &text);
        if let Some(edit) = self.file_edit.as_mut() {
            edit.dirty = files::is_dirty(&base, &text);
        }
        cx.notify();
    }

    /// Writes the edit over the file. Unless `overwrite`, only while the
    /// file still holds the text the edit began from; otherwise nothing is
    /// written and the user is asked what to do.
    fn save_file_edit(
        &mut self,
        overwrite: bool,
        then: AfterSave,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.live_edit().cloned() else {
            return;
        };
        if edit.saving {
            return;
        }
        if !edit.dirty && !overwrite {
            if then == AfterSave::RunScript {
                self.run_in_editor_runtime(edit.script, EditorRun::Script(edit.path), window, cx);
            }
            return;
        }
        let edited = self.file_view.read(cx).value().to_string();
        let saved = files::saved_text(&edit.base, &edited);
        let original = (!overwrite).then(|| edit.base.clone());
        let reply = self.backend.save_project_file(
            edit.project.clone(),
            edit.path.clone(),
            original,
            saved.clone(),
        );
        if let Some(edit) = self.file_edit.as_mut() {
            edit.saving = true;
            edit.problem = None;
        }
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let live = this
                    .live_edit()
                    .is_some_and(|live| live.project == edit.project && live.path == edit.path);
                match result {
                    Ok(wisp_server::desktop::FileSave::Saved) => {
                        // What is still unsaved is measured against what
                        // was just written: text typed meanwhile stays a
                        // draft, the rest is no longer one.
                        let now = if live {
                            Some(this.file_view.read(cx).value().to_string())
                        } else {
                            this.file_drafts
                                .get(&edit.project, &edit.path)
                                .map(|draft| draft.text.clone())
                        };
                        match &now {
                            Some(now) => {
                                this.file_drafts
                                    .note(&edit.project, &edit.path, &saved, now)
                            }
                            None => this.file_drafts.forget(&edit.project, &edit.path),
                        }
                        if live && let Some(open) = this.file_edit.as_mut() {
                            open.dirty = now.is_some_and(|now| files::is_dirty(&saved, &now));
                            open.base = saved;
                            open.saving = false;
                            open.changed_on_disk = false;
                        }
                        if live && then == AfterSave::RunScript {
                            this.run_in_editor_runtime(
                                edit.script,
                                EditorRun::Script(edit.path.clone()),
                                window,
                                cx,
                            );
                        }
                    }
                    Ok(wisp_server::desktop::FileSave::ChangedOnDisk) => {
                        if live && let Some(open) = this.file_edit.as_mut() {
                            open.saving = false;
                            open.changed_on_disk = true;
                            this.layers.open(Layer::FileChangedOnDisk);
                        }
                    }
                    Err(error) => {
                        if live && let Some(open) = this.file_edit.as_mut() {
                            open.saving = false;
                            open.problem = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Throws the edit away and shows the file as it is on disk now.
    fn reload_file_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.live_edit().cloned() else {
            return;
        };
        self.layers.close(Layer::FileChangedOnDisk);
        self.file_drafts.forget(&edit.project, &edit.path);
        // Read again where it is shown, the Files tab or the centre.
        let beside = !self.preview_in_centre();
        self.load_file(edit.path, beside, window, cx);
    }

    /// Runs the selection, or the statement under the caret and then moves
    /// the caret to the next one, as the WebView's Run does.
    fn run_file_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(script) = self.live_edit().map(|edit| edit.script) else {
            return;
        };
        if self.console_busy {
            return;
        }
        // UTF-16 offsets, as a browser's text area gives them.
        let selected = self
            .file_view
            .update(cx, |input, cx| input.selected_text_range(false, window, cx))
            .map(|selection| selection.range)
            .unwrap_or_default();
        let text = self.file_view.read(cx).value().to_string();
        let Some((code, next)) =
            files::run_code(&text, script, selected.start as u32, selected.end as u32)
        else {
            return;
        };
        if let Some((line, column)) = next {
            self.file_view.update(cx, |input, cx| {
                input.set_cursor_position(Position::new(line, column), window, cx)
            });
        }
        self.run_in_editor_runtime(script, EditorRun::Code(code), window, cx);
    }

    /// Saves the edit when there is one, then runs the file whole, so what
    /// runs is exactly what is on disk.
    fn run_file_script(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.live_edit() else {
            return;
        };
        if edit.saving || self.console_busy {
            return;
        }
        self.save_file_edit(false, AfterSave::RunScript, window, cx);
    }

    /// Runs code or the saved script in the conversation's interpreter for
    /// `script`'s language, starting it first if it is not running (the
    /// WebView starts its runtime the same way, on the first run). What it
    /// gives back goes to the console above the composer.
    fn run_in_editor_runtime(
        &mut self,
        script: Script,
        run: EditorRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(project), Some(session)) =
            (self.state.project.clone(), self.state.session.clone())
        else {
            return;
        };
        if self.console_busy {
            return;
        }
        self.console_busy = true;
        if let Some(edit) = self.file_edit.as_mut() {
            edit.problem = None;
        }
        // The console is above the composer: a script run from the centre
        // keeps the conversation beside it, so what it gives back shows.
        self.keep_conversation_beside(window, cx);
        let start = self
            .backend
            .start_runtime(project.clone(), session.clone(), script.runtime());
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let (id, result) = match receive(start).await {
                Ok(id) => {
                    let reply = match &run {
                        EditorRun::Code(code) => backend.run_in_runtime(
                            project.clone(),
                            session.clone(),
                            id.clone(),
                            code.clone(),
                        ),
                        EditorRun::Script(path) => backend.run_script_in_runtime(
                            project.clone(),
                            session.clone(),
                            id.clone(),
                            path.clone(),
                        ),
                    };
                    (Some(id), receive(reply).await)
                }
                Err(error) => (None, Err(error)),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.console_busy = false;
                // The console belongs to the conversation that ran it.
                if this.state.session.as_deref() != Some(session.as_str()) {
                    cx.notify();
                    return;
                }
                match id {
                    Some(id) => {
                        let echo = match &run {
                            EditorRun::Code(code) => code.clone(),
                            EditorRun::Script(path) => files::script_echo(path),
                        };
                        // An error is what the run gave back, as in the
                        // WebView's console.
                        let output =
                            result.unwrap_or_else(|error| wisp_server::desktop::RuntimeOutput {
                                text: crate::i18n::host(&error),
                                plots: Vec::new(),
                            });
                        this.show_console_run(id, &echo, output, window, cx);
                    }
                    // No interpreter to show it in: said in the editor.
                    None => {
                        if let (Some(edit), Err(error)) = (this.file_edit.as_mut(), result) {
                            edit.problem = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Whether the window may close now: not while edits are unsaved,
    /// until the user says to close without them.
    pub fn may_close_window(&mut self, cx: &mut Context<Self>) -> bool {
        if self.close_confirmed || self.file_drafts.is_empty() {
            return true;
        }
        self.layers.open(Layer::ConfirmCloseUnsaved);
        cx.notify();
        false
    }

    /// The editor itself: line numbers, the text to edit, and its keys.
    pub(super) fn render_file_editor(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex_1()
            .min_h_0()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &SaveFile, window, cx| {
                this.save_file_edit(false, AfterSave::Nothing, window, cx)
            }))
            .on_action(cx.listener(|this, _: &RunCode, window, cx| this.run_file_code(window, cx)))
            .on_action(
                cx.listener(|this, _: &RunScript, window, cx| this.run_file_script(window, cx)),
            )
            .child(super::surfaces::input(&self.file_view, cx).h_full())
    }

    /// The row over an edited script: how to run it, Run script, Run, and
    /// Save while there is something to save; or why a script is shown
    /// read-only.
    pub(super) fn render_file_editor_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let border = cx.theme().border;
        let row = || {
            div()
                .h_flex()
                .flex_wrap()
                .gap_2()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(border)
                .text_xs()
        };
        let Some(edit) = self.live_edit() else {
            // A script that cannot be edited says why.
            let refused = self
                .files
                .preview
                .as_ref()
                .and_then(files::editable)
                .and_then(Result::err)?;
            return Some(row().text_color(muted).child(refused));
        };
        let busy = self.console_busy;
        let conversation = self.state.session.is_some();
        let mut bar = row().child(div().flex_1().min_w_0().text_color(muted).child(
            if conversation {
                tr("Ctrl/⌘+Enter runs selection or current statement")
            } else {
                tr("Open a conversation to run this script in its interpreter.")
            },
        ));
        if let Some(problem) = &edit.problem {
            bar = bar.child(
                div()
                    .w_full()
                    .text_color(danger)
                    .child(crate::i18n::host(problem)),
            );
        }
        if edit.changed_on_disk {
            bar = bar
                .child(
                    div()
                        .text_color(danger)
                        .child(tr("The file changed on disk after you began editing it.")),
                )
                .child(
                    Button::new("file-edit-reload")
                        .label(tr("Reload from disk"))
                        .xsmall()
                        .ghost()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.reload_file_edit(window, cx)),
                        ),
                );
        }
        bar = bar
            .child(
                Button::new("file-edit-run-script")
                    .icon(glyph("file-play"))
                    .label(tr("Run script"))
                    .xsmall()
                    .ghost()
                    .disabled(busy || edit.saving || !conversation)
                    .tooltip(tr(
                        "Save, then run the whole script in the conversation's interpreter \
                         (Ctrl/⌘+Shift+Enter). Loaded data stays in memory.",
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.run_file_script(window, cx))),
            )
            .child(
                Button::new("file-edit-run")
                    .icon(glyph("play"))
                    .label(if busy { tr("Running…") } else { tr("Run") })
                    .xsmall()
                    .primary()
                    .disabled(busy || !conversation)
                    .tooltip(tr("Run selection or current statement (Ctrl/⌘+Enter)"))
                    .on_click(cx.listener(|this, _, window, cx| this.run_file_code(window, cx))),
            );
        if edit.dirty || edit.problem.is_some() {
            bar = bar.child(
                Button::new("file-edit-save")
                    .icon(glyph("save"))
                    .label(if edit.saving {
                        tr("Saving…")
                    } else {
                        tr("Save")
                    })
                    .xsmall()
                    .disabled(edit.saving)
                    .tooltip(tr("Save the file (Ctrl/⌘+S)"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.save_file_edit(false, AfterSave::Nothing, window, cx)
                    })),
            );
        }
        Some(bar)
    }

    /// The question when a save found the file changed on disk.
    pub(super) fn render_file_changed(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self
            .file_edit
            .as_ref()
            .map(|edit| files::file_name(&edit.path).to_owned())
            .unwrap_or_default();
        surfaces::dialog(
            "file-changed",
            DialogWidth::Narrow,
            tr("The file changed on disk"),
        )
        .child(div().text_sm().child(crate::i18n::fill(
            tr(
                "{} was changed after you began editing it, by the assistant or another \
                program. Saving your edit now would replace that change.",
            ),
            &[&name],
        )))
        .action(
            Button::new("file-changed-keep")
                .label(tr("Keep editing"))
                .small()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::FileChangedOnDisk);
                    cx.notify();
                })),
        )
        .action(
            Button::new("file-changed-reload")
                .label(tr("Reload from disk"))
                .small()
                .tooltip(tr("Your edit is thrown away"))
                .on_click(cx.listener(|this, _, window, cx| this.reload_file_edit(window, cx))),
        )
        .action(
            Button::new("file-changed-overwrite")
                .label(tr("Overwrite"))
                .small()
                .danger()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.layers.close(Layer::FileChangedOnDisk);
                    this.save_file_edit(true, AfterSave::Nothing, window, cx);
                })),
        )
    }

    /// The question before the window closes over unsaved edits.
    pub(super) fn render_close_unsaved(&self, cx: &mut Context<Self>) -> impl IntoElement {
        surfaces::dialog(
            "close-unsaved",
            DialogWidth::Narrow,
            tr("Close without saving?"),
        )
        .child(
            div()
                .text_sm()
                .child(files::unsaved_note(&self.file_drafts.names())),
        )
        .action(
            Button::new("close-unsaved-keep")
                .label(tr("Keep editing"))
                .small()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.layers.close(Layer::ConfirmCloseUnsaved);
                    cx.notify();
                })),
        )
        .action(
            Button::new("close-unsaved-close")
                .label(tr("Close without saving"))
                .small()
                .danger()
                .on_click(cx.listener(|this, _, window, _| {
                    this.close_confirmed = true;
                    this.layers.close(Layer::ConfirmCloseUnsaved);
                    window.remove_window();
                })),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui::KeyContext;

    fn stack(names: &[&str]) -> Vec<KeyContext> {
        names
            .iter()
            .map(|name| KeyContext::parse(name).unwrap())
            .collect()
    }

    /// Save and the two runs belong to the editor's input alone: the
    /// composer keeps its own Cmd/Ctrl+Return, and the other inputs theirs.
    #[test]
    fn the_editor_keys_apply_only_in_the_editor() {
        let bindings = super::file_editor_bindings();
        assert_eq!(bindings.len(), 3);
        let editor = stack(&["Workspace", "FileEditor", "Input"]);
        let composer = stack(&["Workspace", "Composer", "Input"]);
        let elsewhere = stack(&["Workspace", "Input"]);
        for binding in &bindings {
            let applies = binding.predicate().unwrap();
            assert!(applies.eval_inner(&editor, &editor));
            assert!(!applies.eval_inner(&composer, &composer));
            assert!(!applies.eval_inner(&elsewhere, &elsewhere));
        }
    }
}
