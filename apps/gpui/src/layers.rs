//! Window-level Escape stack, ordered from the bottom surface to the topmost.
//! The stack is the single source of truth for which dismissible layers are
//! open, so one Escape press can only ever close the topmost one. It holds no
//! graphics state and is exercised without a window.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Project file browser docked beside the transcript.
    FilesPanel,
    /// File preview shown over the browser's directory listing.
    FilePreview,
    /// Settings, shown in place of the transcript.
    Settings,
    /// Inline title editor for the open conversation.
    Rename,
    /// Confirmation before a conversation is permanently deleted.
    ConfirmDelete,
    /// Confirmation on the home page before a project leaves Wisp.
    ConfirmRemoveProject,
    /// Command palette, drawn over everything else.
    Palette,
    /// Question before a conversation stops asking for approvals.
    ConfirmFullAccess,
    /// Reasoning-effort choices above the composer.
    EffortMenu,
    /// Model choices above the composer.
    ModelMenu,
    /// Where the conversation's work runs, above the composer.
    RunsOnMenu,
    /// The expert a new conversation is started as, above the composer.
    ExpertMenu,
    /// The slash commands, above the composer.
    SlashMenu,
    /// The quick actions, above the composer.
    QuickMenu,
    /// The dialog for a project's name, description and rules.
    ProjectSettings,
    /// The find bar over the open conversation.
    Find,
    /// The outline of the open conversation, under its header.
    Outline,
    /// The research journey of the open project: a page of its own.
    Journey,
    /// The form on the journey page for a record of the researcher's own.
    JournalForm,
    /// The scheduled tasks of every project: a page of its own.
    Schedules,
    /// The form on that page for a new task.
    ScheduleForm,
    /// Question before a scheduled task is deleted.
    ConfirmDeleteSchedule,
    /// The papers of the open project: a page of its own.
    Papers,
    /// The form on the papers page for a new paper or a new name.
    PaperForm,
    /// The form for a figure, table or claim of a paper.
    PaperItemForm,
    /// The results and runs to choose evidence from.
    PaperSources,
    /// Question before a paper is removed.
    ConfirmRemovePaper,
    /// The research calendar across projects: a page of its own.
    Calendar,
    /// The overview of what Wisp can draw on in the open project.
    Capabilities,
    /// The trajectory of the open conversation: a dialog over it.
    Trajectory,
    /// The review of the open conversation: a dialog over it.
    Review,
    /// What the model is sent for the open conversation: a dialog over it.
    ModelView,
    /// The research archive of the open conversation: a dialog over it.
    ResearchArchive,
    /// The name field in the Files panel: a new folder or file, or a rename.
    FileName,
    /// Question in the Files panel before an entry goes to the trash.
    ConfirmTrashFile,
    /// Confirmation in Settings before a saved model is removed.
    ConfirmDeleteModel,
    /// Sort and grouping choices under the conversation list's heading.
    SortMenu,
    /// Select mode: sidebar rows are picked instead of opened.
    SelectMode,
    /// Groups the picked conversations can be moved to.
    BatchGroupMenu,
    /// Confirmation before the picked conversations are deleted.
    ConfirmBatchDelete,
    /// Name field for a new group or one being renamed.
    FolderName,
    /// Actions for one group, shown under its heading.
    FolderMenu,
    /// Confirmation before a group is deleted.
    ConfirmDeleteFolder,
    /// Groups the open conversation can be put in.
    GroupMenu,
    /// Projects the open conversation can be copied or moved to.
    TransferMenu,
    /// Question before a conversation is cut back to an earlier message.
    ConfirmRewind,
    /// The memo being written or changed, in place of the memo list.
    MemoEditor,
    /// Question before a memo is deleted.
    ConfirmDeleteMemo,
    /// The habit, note or remembered text being written, in place of the
    /// Memory tab's lists.
    MemoryEditor,
    /// Question before a habit or a note is deleted.
    ConfirmDeleteMemory,
    /// The Favorites page, shown in place of the rest of the window.
    Favorites,
    /// Question on the Favorites page before a saved item is removed.
    ConfirmRemoveFavorite,
    /// The form for an MCP server, on the Connections page of Settings.
    McpForm,
    /// Question before an MCP server is removed.
    ConfirmRemoveMcp,
    /// Question before the remote access code is replaced, in Settings.
    ConfirmResetRemoteCode,
    /// Question before a plugin is removed, on its page of Settings.
    ConfirmRemovePlugin,
    /// The form for a workflow, on its page of Settings.
    WorkflowForm,
    /// Question before a workflow is removed.
    ConfirmRemoveWorkflow,
    /// The form for a quick action, on its page of Settings.
    QuickActionForm,
    /// Question before a quick action is removed.
    ConfirmRemoveQuickAction,
    /// The form for an expert, on the Experts page of Settings.
    ExpertForm,
    /// Question before an expert is removed.
    ConfirmRemoveExpert,
    /// The form for an SSH host, on the Environments page of Settings.
    EnvironmentForm,
    /// Question before an SSH host is removed.
    ConfirmRemoveEnvironment,
}

impl Layer {
    /// Menus that open above the composer; only one shows at a time.
    pub const COMPOSER_MENUS: [Layer; 6] = [
        Layer::EffortMenu,
        Layer::ModelMenu,
        Layer::RunsOnMenu,
        Layer::ExpertMenu,
        Layer::QuickMenu,
        Layer::SlashMenu,
    ];
    /// What the row under the conversation's title can turn into, one at a
    /// time.
    pub const CONVERSATION_BAR: [Layer; 5] = [
        Layer::Rename,
        Layer::ConfirmDelete,
        Layer::GroupMenu,
        Layer::TransferMenu,
        Layer::ConfirmRewind,
    ];
    /// Small surfaces of the sidebar's conversation list, one at a time.
    /// Select mode is not one of them: it is a mode they can open inside.
    pub const SIDEBAR: [Layer; 6] = [
        Layer::SortMenu,
        Layer::BatchGroupMenu,
        Layer::ConfirmBatchDelete,
        Layer::FolderName,
        Layer::FolderMenu,
        Layer::ConfirmDeleteFolder,
    ];
}

impl Layer {
    /// The surface this one is drawn inside, which it cannot outlive.
    fn parent(self) -> Option<Layer> {
        match self {
            Layer::FilePreview
            | Layer::FileName
            | Layer::ConfirmTrashFile
            | Layer::MemoEditor
            | Layer::ConfirmDeleteMemo
            | Layer::MemoryEditor
            | Layer::ConfirmDeleteMemory => Some(Layer::FilesPanel),
            Layer::ConfirmDeleteModel
            | Layer::McpForm
            | Layer::ConfirmRemoveMcp
            | Layer::EnvironmentForm
            | Layer::ConfirmRemoveEnvironment
            | Layer::ExpertForm
            | Layer::ConfirmRemoveExpert
            | Layer::QuickActionForm
            | Layer::WorkflowForm
            | Layer::ConfirmRemoveWorkflow
            | Layer::ConfirmRemovePlugin
            | Layer::ConfirmResetRemoteCode
            | Layer::ConfirmRemoveQuickAction => Some(Layer::Settings),
            Layer::ConfirmRemoveFavorite => Some(Layer::Favorites),
            Layer::JournalForm => Some(Layer::Journey),
            Layer::ScheduleForm | Layer::ConfirmDeleteSchedule => Some(Layer::Schedules),
            Layer::PaperForm
            | Layer::PaperItemForm
            | Layer::PaperSources
            | Layer::ConfirmRemovePaper => Some(Layer::Papers),
            Layer::BatchGroupMenu | Layer::ConfirmBatchDelete => Some(Layer::SelectMode),
            _ => None,
        }
    }

    fn within(self, ancestor: Layer) -> bool {
        self == ancestor || self.parent().is_some_and(|parent| parent.within(ancestor))
    }
}

#[derive(Default)]
pub struct EscapeStack(Vec<Layer>);

impl EscapeStack {
    /// Opens `layer` as the topmost surface. Reopening moves it to the top.
    pub fn open(&mut self, layer: Layer) {
        self.0.retain(|open| *open != layer);
        self.0.push(layer);
    }

    /// Closes `layer` and the surfaces drawn inside it, wherever they sit in
    /// the stack. Unrelated layers stay open. Returns whether it was open.
    pub fn close(&mut self, layer: Layer) -> bool {
        let open = self.0.contains(&layer);
        self.0.retain(|other| !other.within(layer));
        open
    }

    /// Opens `layer` in place of whichever other member of `set` is open.
    pub fn switch(&mut self, layer: Layer, set: &[Layer]) {
        for other in set {
            if *other != layer {
                self.close(*other);
            }
        }
        self.open(layer);
    }

    /// The member of `set` that is drawn over the others, if any is open.
    pub fn top_of(&self, set: &[Layer]) -> Option<Layer> {
        self.0.iter().rev().copied().find(|open| set.contains(open))
    }

    /// Closes only the topmost surface and reports which one it was.
    pub fn dismiss_top(&mut self) -> Option<Layer> {
        self.0.pop()
    }

    pub fn contains(&self, layer: Layer) -> bool {
        self.0.contains(&layer)
    }
}

/// A bare Escape press. Modified chords stay available to platform shortcuts.
pub fn is_escape(key: &str, modified: bool) -> bool {
    key == "escape" && !modified
}

/// Window-wide commands that work wherever focus is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    NewConversation,
    Settings,
    Palette,
    Find,
    Sidebar,
}

/// `primary` is Cmd on macOS and Ctrl elsewhere, held alone: adding Shift,
/// Alt or the other platform key leaves the chord to whoever else wants it.
pub fn shortcut(key: &str, primary: bool, others: bool) -> Option<Shortcut> {
    if !primary || others {
        return None;
    }
    match key {
        "n" => Some(Shortcut::NewConversation),
        "," => Some(Shortcut::Settings),
        "k" => Some(Shortcut::Palette),
        "f" => Some(Shortcut::Find),
        "b" => Some(Shortcut::Sidebar),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_escape_closes_only_the_topmost_layer() {
        let mut stack = EscapeStack::default();
        assert_eq!(stack.dismiss_top(), None);
        stack.open(Layer::FilesPanel);
        stack.open(Layer::FilePreview);
        assert_eq!(stack.dismiss_top(), Some(Layer::FilePreview));
        assert!(stack.contains(Layer::FilesPanel));
        assert!(!stack.contains(Layer::FilePreview));
        assert_eq!(stack.dismiss_top(), Some(Layer::FilesPanel));
        assert!(!stack.contains(Layer::FilesPanel));
        assert_eq!(stack.dismiss_top(), None);
    }

    #[test]
    fn closing_a_parent_closes_its_children_and_reopening_does_not_duplicate() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::FilePreview);
        stack.open(Layer::FilePreview);
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::FilesPanel) && !stack.contains(Layer::FilePreview));
        assert!(!stack.close(Layer::FilesPanel));

        stack.open(Layer::FilesPanel);
        stack.open(Layer::FilePreview);
        assert!(stack.close(Layer::FilePreview));
        assert!(stack.contains(Layer::FilesPanel));
        assert_eq!(stack.dismiss_top(), Some(Layer::FilesPanel));
    }

    #[test]
    fn closing_a_layer_leaves_unrelated_layers_open_in_order() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::Rename);
        stack.open(Layer::FilePreview);
        stack.open(Layer::ConfirmDelete);
        // The panel takes its preview with it, not the conversation layers.
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::FilePreview));
        assert!(stack.contains(Layer::Rename) && stack.contains(Layer::ConfirmDelete));
        assert!(!stack.close(Layer::Settings));
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDelete));
        assert_eq!(stack.dismiss_top(), Some(Layer::Rename));
        assert_eq!(stack.dismiss_top(), None);
    }

    #[test]
    fn only_a_bare_escape_dismisses() {
        assert!(is_escape("escape", false));
        assert!(!is_escape("escape", true));
        assert!(!is_escape("enter", false));
    }

    #[test]
    fn shortcuts_need_exactly_the_platform_modifier() {
        assert_eq!(shortcut("n", true, false), Some(Shortcut::NewConversation));
        assert_eq!(shortcut(",", true, false), Some(Shortcut::Settings));
        // Typing the letter, or a longer chord, is not the command.
        assert_eq!(shortcut("n", false, false), None);
        assert_eq!(shortcut("n", true, true), None);
        assert_eq!(shortcut("k", true, false), Some(Shortcut::Palette));
        assert_eq!(shortcut("f", true, false), Some(Shortcut::Find));
        assert_eq!(shortcut("b", true, false), Some(Shortcut::Sidebar));
        assert_eq!(shortcut("b", false, false), None);
        // With Shift it is someone else's chord.
        assert_eq!(shortcut("f", true, true), None);
        assert_eq!(shortcut("j", true, false), None);
    }

    #[test]
    fn the_palette_closes_first_and_leaves_what_is_under_it() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::Settings);
        stack.open(Layer::Palette);
        assert_eq!(stack.dismiss_top(), Some(Layer::Palette));
        assert!(stack.contains(Layer::Settings) && stack.contains(Layer::FilesPanel));
        // Reopening it over a layer opened meanwhile keeps it topmost.
        stack.open(Layer::Palette);
        stack.open(Layer::ConfirmDelete);
        stack.open(Layer::Palette);
        assert_eq!(stack.dismiss_top(), Some(Layer::Palette));
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDelete));
    }

    #[test]
    fn composer_layers_close_one_at_a_time_and_leave_the_panel() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::EffortMenu);
        assert_eq!(stack.dismiss_top(), Some(Layer::EffortMenu));
        stack.open(Layer::ConfirmFullAccess);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmFullAccess));
        assert!(stack.contains(Layer::FilesPanel));
        assert!(!stack.close(Layer::EffortMenu));
    }

    #[test]
    fn select_mode_holds_its_menus_and_bar_surfaces_replace_each_other() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::SelectMode);
        stack.switch(Layer::BatchGroupMenu, &Layer::SIDEBAR);
        stack.switch(Layer::ConfirmBatchDelete, &Layer::SIDEBAR);
        assert!(!stack.contains(Layer::BatchGroupMenu));
        // Escape answers the question first; select mode stays.
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmBatchDelete));
        assert!(stack.contains(Layer::SelectMode));
        // Leaving select mode takes whatever it had open with it.
        stack.open(Layer::BatchGroupMenu);
        assert!(stack.close(Layer::SelectMode));
        assert!(!stack.contains(Layer::BatchGroupMenu));

        // A group's menu is not part of select mode and outlives it.
        stack.open(Layer::SelectMode);
        stack.switch(Layer::FolderMenu, &Layer::SIDEBAR);
        assert!(stack.close(Layer::SelectMode));
        assert_eq!(stack.dismiss_top(), Some(Layer::FolderMenu));

        stack.switch(Layer::Rename, &Layer::CONVERSATION_BAR);
        stack.switch(Layer::ConfirmRewind, &Layer::CONVERSATION_BAR);
        stack.switch(Layer::TransferMenu, &Layer::CONVERSATION_BAR);
        assert!(!stack.contains(Layer::Rename) && !stack.contains(Layer::ConfirmRewind));
        stack.switch(Layer::SortMenu, &Layer::SIDEBAR);
        assert_eq!(stack.dismiss_top(), Some(Layer::SortMenu));
        assert_eq!(stack.dismiss_top(), Some(Layer::TransferMenu));
        assert_eq!(stack.dismiss_top(), None);
    }

    #[test]
    fn the_memo_editor_lives_and_dies_with_the_side_panel() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::MemoEditor);
        // Escape gives up the edit, not the panel.
        assert_eq!(stack.dismiss_top(), Some(Layer::MemoEditor));
        assert!(stack.contains(Layer::FilesPanel));
        stack.open(Layer::ConfirmDeleteMemo);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDeleteMemo));
        stack.open(Layer::MemoEditor);
        stack.open(Layer::MemoryEditor);
        stack.open(Layer::ConfirmDeleteMemory);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDeleteMemory));
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::MemoEditor) && !stack.contains(Layer::MemoryEditor));
    }

    #[test]
    fn of_two_pages_the_one_opened_last_is_shown_and_closes_first() {
        let pages = [Layer::Settings, Layer::Favorites];
        let mut stack = EscapeStack::default();
        assert_eq!(stack.top_of(&pages), None);
        stack.open(Layer::FilesPanel);
        stack.open(Layer::Favorites);
        stack.open(Layer::ConfirmRemoveFavorite);
        assert_eq!(stack.top_of(&pages), Some(Layer::Favorites));
        stack.open(Layer::Settings);
        assert_eq!(stack.top_of(&pages), Some(Layer::Settings));
        // Escape leaves Settings and is back on Favorites, question and all.
        assert_eq!(stack.dismiss_top(), Some(Layer::Settings));
        assert_eq!(stack.top_of(&pages), Some(Layer::Favorites));
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmRemoveFavorite));
        stack.open(Layer::ConfirmRemoveFavorite);
        assert!(stack.close(Layer::Favorites));
        assert!(!stack.contains(Layer::ConfirmRemoveFavorite));
        assert!(stack.contains(Layer::FilesPanel));
    }

    #[test]
    fn the_mcp_form_is_part_of_settings_and_escape_leaves_it_first() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::Settings);
        stack.open(Layer::McpForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::McpForm));
        assert!(stack.contains(Layer::Settings));
        stack.open(Layer::ConfirmRemoveMcp);
        stack.open(Layer::EnvironmentForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::EnvironmentForm));
        stack.open(Layer::ConfirmRemoveEnvironment);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::ConfirmRemoveMcp));
        assert!(!stack.contains(Layer::ConfirmRemoveEnvironment));
    }

    #[test]
    fn closing_settings_takes_its_delete_question_with_it() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::Settings);
        stack.open(Layer::ConfirmDeleteModel);
        // Escape answers the question first and leaves Settings open.
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDeleteModel));
        assert!(stack.contains(Layer::Settings));
        stack.open(Layer::ConfirmDeleteModel);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::ConfirmDeleteModel));
        // The expert form is inside Settings: Escape leaves it first, and
        // closing Settings takes it along.
        stack.open(Layer::Settings);
        stack.open(Layer::ExpertForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::ExpertForm));
        stack.open(Layer::ConfirmRemoveExpert);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::ConfirmRemoveExpert));
        // And the question before the remote access code is replaced.
        stack.open(Layer::Settings);
        stack.open(Layer::ConfirmResetRemoteCode);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmResetRemoteCode));
        assert!(stack.contains(Layer::Settings));
        // The question before a plugin is removed is inside Settings too.
        stack.open(Layer::Settings);
        stack.open(Layer::ConfirmRemovePlugin);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmRemovePlugin));
        assert!(stack.contains(Layer::Settings));
        stack.open(Layer::ConfirmRemovePlugin);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::ConfirmRemovePlugin));
        // The workflow form and its remove question are inside Settings.
        stack.open(Layer::Settings);
        stack.open(Layer::WorkflowForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::WorkflowForm));
        stack.open(Layer::ConfirmRemoveWorkflow);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmRemoveWorkflow));
        assert!(stack.contains(Layer::Settings));
        stack.open(Layer::WorkflowForm);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::WorkflowForm));
        // So is the quick action form, and the question before removing one.
        stack.open(Layer::Settings);
        stack.open(Layer::QuickActionForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::QuickActionForm));
        assert!(stack.contains(Layer::Settings));
        stack.open(Layer::ConfirmRemoveQuickAction);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmRemoveQuickAction));
        stack.open(Layer::QuickActionForm);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::QuickActionForm));
        // The composer menus are ordinary layers of their own.
        for menu in Layer::COMPOSER_MENUS {
            stack.open(menu);
            assert_eq!(stack.dismiss_top(), Some(menu));
        }
        // The Files panel's name field and trash question are inside the
        // panel: Escape closes them alone, and closing the panel takes them.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::FileName);
        assert_eq!(stack.dismiss_top(), Some(Layer::FileName));
        assert!(stack.contains(Layer::FilesPanel));
        stack.open(Layer::ConfirmTrashFile);
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::ConfirmTrashFile));
        // The calendar is a page like any other: Escape closes it alone.
        stack.open(Layer::Calendar);
        assert_eq!(stack.dismiss_top(), Some(Layer::Calendar));
        // The journey is a page, and its form is inside it.
        stack.open(Layer::Journey);
        stack.open(Layer::JournalForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::JournalForm));
        assert!(stack.contains(Layer::Journey));
        stack.open(Layer::JournalForm);
        assert!(stack.close(Layer::Journey));
        assert!(!stack.contains(Layer::JournalForm));
        // The scheduled tasks page has a form and a question inside it.
        stack.open(Layer::Schedules);
        stack.open(Layer::ScheduleForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::ScheduleForm));
        stack.open(Layer::ConfirmDeleteSchedule);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDeleteSchedule));
        assert!(stack.contains(Layer::Schedules));
        stack.open(Layer::ScheduleForm);
        assert!(stack.close(Layer::Schedules));
        assert!(!stack.contains(Layer::ScheduleForm));
        // The papers page is the same: a form or a question inside it
        // goes first, and closing the page takes them along.
        stack.open(Layer::Papers);
        for inside in [
            Layer::PaperForm,
            Layer::PaperItemForm,
            Layer::PaperSources,
            Layer::ConfirmRemovePaper,
        ] {
            stack.open(inside);
            assert_eq!(stack.dismiss_top(), Some(inside));
            assert!(stack.contains(Layer::Papers));
        }
        stack.open(Layer::PaperSources);
        assert!(stack.close(Layer::Papers));
        assert!(!stack.contains(Layer::PaperSources));
        // The trajectory is a dialog over the conversation: Escape closes
        // it alone, and a panel that was open beside it stays.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::Trajectory);
        assert_eq!(stack.dismiss_top(), Some(Layer::Trajectory));
        assert!(stack.contains(Layer::FilesPanel));
        assert!(stack.close(Layer::FilesPanel));
        // So is the research archive.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::ResearchArchive);
        assert_eq!(stack.dismiss_top(), Some(Layer::ResearchArchive));
        assert!(stack.close(Layer::FilesPanel));
        // So is the list of what the model is sent.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::ModelView);
        assert_eq!(stack.dismiss_top(), Some(Layer::ModelView));
        assert!(stack.close(Layer::FilesPanel));
        // So is the review.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::Review);
        assert_eq!(stack.dismiss_top(), Some(Layer::Review));
        assert!(stack.close(Layer::FilesPanel));
        // The project dialog opens over whatever is showing; Escape closes
        // it alone, and what was under it is still there.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::ProjectSettings);
        assert_eq!(stack.dismiss_top(), Some(Layer::ProjectSettings));
        assert!(stack.contains(Layer::FilesPanel));
    }
}
