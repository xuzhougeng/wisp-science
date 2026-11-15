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
    /// The composer's choice of who runs a new conversation: Wisp or an
    /// ACP agent.
    AgentMenu,
    /// The list of one of the ACP agent's modes or options.
    AcpSettingMenu,
    /// The slash commands, above the composer.
    SlashMenu,
    /// The quick actions, above the composer.
    QuickMenu,
    /// The dialog for a project's name, description and rules.
    ProjectSettings,
    /// Question before a sync conflict is ended, inside project settings.
    ConfirmSyncReplace,
    /// The dialog that joins a project from another device by its code.
    JoinSyncedProject,
    /// The dialog that names a new project before its folder is made.
    NewProject,
    /// The example conversations that ship with Wisp, read only: a dialog
    /// over the home page.
    Examples,
    /// The projects the example being read can be copied into.
    ExampleCopy,
    /// The dialog that names an exploration before it starts.
    StartExploration,
    /// What keeping an exploration would bring in.
    ExplorationPreview,
    /// Question before an exploration or a round is given up.
    ConfirmExploration,
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
    /// A console on one of the conversation's interpreters, over the
    /// composer.
    RuntimeConsole,
    /// The token budget of a delegated task being changed before its batch
    /// runs again, in the Agents tab.
    TaskBudget,
    /// The research mandates of every project: a page of its own.
    Mandates,
    /// The form on that page for a new mandate or a change to one.
    MandateForm,
    /// Question before a mandate is deleted.
    ConfirmDeleteMandate,
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
    /// The readiness check of the shown draft version, and finalizing it.
    PaperFinalize,
    /// The reason a finding of that check may stand.
    PaperWaiver,
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
    /// Bringing the open branch back into its source: a dialog over it.
    BranchMerge,
    /// Conversations from Codex or Claude Code to import: a dialog.
    ImportConversations,
    /// The first-run guide, over whatever the first screen is.
    Onboarding,
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
    /// The address and checksum of a plugin to download, on its page.
    PluginAddress,
    /// The form for a workflow, on its page of Settings.
    WorkflowForm,
    /// Question before a workflow is removed.
    ConfirmRemoveWorkflow,
    /// Writing a workflow from skills, on the Workflows page of Settings.
    WorkflowFromSkills,
    /// The SSH hosts a terminal can be opened on, over the terminal dock.
    TerminalHosts,
    /// A skill's tags being changed, on the Skills page of Settings.
    SkillTags,
    /// The address of skills on GitHub, on the Skills page of Settings.
    SkillFromGithub,
    /// The form for a quick action, on its page of Settings.
    QuickActionForm,
    /// The form for a command hook, on the Hooks page of Settings.
    HookForm,
    /// Question before a quick action is removed.
    ConfirmRemoveQuickAction,
    /// The form for an expert, on the Experts page of Settings.
    ExpertForm,
    /// The form for an ACP agent, on the ACP agents page of Settings.
    AcpAgentForm,
    /// Question before an ACP agent is removed from the list.
    ConfirmRemoveAcpAgent,
    /// Question before a ChatGPT (Codex) account is removed from the pool.
    ConfirmRemoveCodexAccount,
    /// Question before WeChat is unbound, on the Chat apps page.
    ConfirmUnbindWeixin,
    /// Question before the research assistant's WeChat is unbound.
    ConfirmUnbindAssistantWeixin,
    /// Question before Feishu is unbound, on the same page.
    ConfirmUnbindFeishu,
    /// Question before this computer leaves its Wisp Lab.
    ConfirmLeaveLab,
    /// Question before the leader removes a lab member.
    ConfirmRemoveLabMember,
    /// Question before an expert is removed.
    ConfirmRemoveExpert,
    /// The form for an SSH host, on the Environments page of Settings.
    EnvironmentForm,
    /// Question before an SSH host is removed.
    ConfirmRemoveEnvironment,
}

impl Layer {
    /// Menus that open above the composer; only one shows at a time.
    pub const COMPOSER_MENUS: [Layer; 8] = [
        Layer::EffortMenu,
        Layer::ModelMenu,
        Layer::RunsOnMenu,
        Layer::ExpertMenu,
        Layer::AgentMenu,
        Layer::AcpSettingMenu,
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
            | Layer::ConfirmDeleteMemory
            | Layer::TaskBudget => Some(Layer::FilesPanel),
            Layer::ConfirmDeleteModel
            | Layer::McpForm
            | Layer::ConfirmRemoveMcp
            | Layer::EnvironmentForm
            | Layer::ConfirmRemoveEnvironment
            | Layer::ExpertForm
            | Layer::ConfirmRemoveExpert
            | Layer::AcpAgentForm
            | Layer::ConfirmRemoveAcpAgent
            | Layer::ConfirmRemoveCodexAccount
            | Layer::ConfirmUnbindWeixin
            | Layer::ConfirmUnbindAssistantWeixin
            | Layer::ConfirmUnbindFeishu
            | Layer::ConfirmLeaveLab
            | Layer::ConfirmRemoveLabMember
            | Layer::QuickActionForm
            | Layer::HookForm
            | Layer::WorkflowForm
            | Layer::ConfirmRemoveWorkflow
            | Layer::WorkflowFromSkills
            | Layer::SkillTags
            | Layer::SkillFromGithub
            | Layer::ConfirmRemovePlugin
            | Layer::PluginAddress
            | Layer::ConfirmResetRemoteCode
            | Layer::ConfirmRemoveQuickAction => Some(Layer::Settings),
            Layer::ConfirmRemoveFavorite => Some(Layer::Favorites),
            Layer::ConfirmSyncReplace => Some(Layer::ProjectSettings),
            Layer::JournalForm => Some(Layer::Journey),
            Layer::ScheduleForm | Layer::ConfirmDeleteSchedule => Some(Layer::Schedules),
            Layer::MandateForm | Layer::ConfirmDeleteMandate => Some(Layer::Mandates),
            Layer::PaperForm
            | Layer::PaperItemForm
            | Layer::PaperSources
            | Layer::ConfirmRemovePaper
            | Layer::PaperFinalize => Some(Layer::Papers),
            Layer::PaperWaiver => Some(Layer::PaperFinalize),
            Layer::BatchGroupMenu | Layer::ConfirmBatchDelete => Some(Layer::SelectMode),
            Layer::ExampleCopy => Some(Layer::Examples),
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

/// Whether a terminal that has focus keeps a key for its shell rather
/// than the window taking it: Escape, which the programs in it read, and,
/// off macOS, every Ctrl chord (Ctrl+C, Ctrl+K and the rest belong to the
/// shell there). The window's Cmd shortcuts on macOS stay the window's.
pub fn terminal_keeps(key: &str, control: bool, mac: bool) -> bool {
    key == "escape" || (control && !mac)
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
    /// Search the project's conversations, their messages included.
    SearchHistory,
    /// The palette with only its commands.
    Commands,
}

/// `primary` is Cmd on macOS and Ctrl elsewhere. Shift makes a chord of its
/// own; Alt or the other platform key leave the chord to whoever else
/// wants it.
pub fn shortcut(key: &str, primary: bool, shift: bool, others: bool) -> Option<Shortcut> {
    if !primary || others {
        return None;
    }
    if shift {
        return (key == "f").then_some(Shortcut::SearchHistory);
    }
    match key {
        "n" => Some(Shortcut::NewConversation),
        "," => Some(Shortcut::Settings),
        "k" => Some(Shortcut::Palette),
        "p" => Some(Shortcut::Commands),
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
        assert_eq!(
            shortcut("n", true, false, false),
            Some(Shortcut::NewConversation)
        );
        assert_eq!(shortcut(",", true, false, false), Some(Shortcut::Settings));
        // Typing the letter, or a longer chord, is not the command.
        assert_eq!(shortcut("n", false, false, false), None);
        assert_eq!(shortcut("n", true, false, true), None);
        assert_eq!(shortcut("k", true, false, false), Some(Shortcut::Palette));
        assert_eq!(shortcut("p", true, false, false), Some(Shortcut::Commands));
        assert_eq!(shortcut("f", true, false, false), Some(Shortcut::Find));
        assert_eq!(shortcut("b", true, false, false), Some(Shortcut::Sidebar));
        assert_eq!(shortcut("b", false, false, false), None);
        // With Shift, F searches the project's history; the rest are
        // someone else's chords.
        assert_eq!(
            shortcut("f", true, true, false),
            Some(Shortcut::SearchHistory)
        );
        assert_eq!(shortcut("n", true, true, false), None);
        assert_eq!(shortcut("f", false, true, false), None);
        assert_eq!(shortcut("f", true, true, true), None);
        assert_eq!(shortcut("j", true, false, false), None);
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
        // The ACP agent form and its remove question are the same.
        stack.open(Layer::Settings);
        for inside in [Layer::AcpAgentForm, Layer::ConfirmRemoveAcpAgent] {
            stack.open(inside);
            assert_eq!(stack.dismiss_top(), Some(inside));
            assert!(stack.contains(Layer::Settings));
        }
        stack.open(Layer::AcpAgentForm);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::AcpAgentForm));
        // So are the questions before WeChat and Feishu are unbound.
        for question in [
            Layer::ConfirmUnbindWeixin,
            Layer::ConfirmUnbindAssistantWeixin,
            Layer::ConfirmUnbindFeishu,
            Layer::ConfirmLeaveLab,
            Layer::ConfirmRemoveLabMember,
        ] {
            stack.open(Layer::Settings);
            stack.open(question);
            assert_eq!(stack.dismiss_top(), Some(question));
            assert!(stack.contains(Layer::Settings));
            stack.open(question);
            assert!(stack.close(Layer::Settings));
            assert!(!stack.contains(question));
        }
        // And the question before a Codex account is removed.
        stack.open(Layer::Settings);
        stack.open(Layer::ConfirmRemoveCodexAccount);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmRemoveCodexAccount));
        assert!(stack.contains(Layer::Settings));
        stack.open(Layer::ConfirmRemoveCodexAccount);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::ConfirmRemoveCodexAccount));
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
        // Writing one from skills is inside Settings too: Escape closes it
        // and leaves Settings open.
        stack.open(Layer::Settings);
        stack.open(Layer::WorkflowFromSkills);
        assert_eq!(stack.dismiss_top(), Some(Layer::WorkflowFromSkills));
        assert!(stack.contains(Layer::Settings));
        stack.open(Layer::WorkflowFromSkills);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::WorkflowFromSkills));
        // A skill's tag editor is inside Settings.
        stack.open(Layer::Settings);
        stack.open(Layer::SkillTags);
        assert_eq!(stack.dismiss_top(), Some(Layer::SkillTags));
        assert!(stack.contains(Layer::Settings));
        assert!(stack.close(Layer::Settings));
        // The terminal's host menu stands on its own: Escape closes it.
        stack.open(Layer::TerminalHosts);
        assert_eq!(stack.dismiss_top(), Some(Layer::TerminalHosts));
        // A terminal with focus keeps Escape and, off macOS, Ctrl chords;
        // Cmd shortcuts on macOS stay the window's.
        assert!(terminal_keeps("escape", false, true));
        assert!(terminal_keeps("k", true, false));
        assert!(!terminal_keeps("k", true, true));
        assert!(!terminal_keeps("k", false, false));
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
        // An interpreter's console over the composer closes on its own.
        stack.open(Layer::RuntimeConsole);
        assert_eq!(stack.dismiss_top(), Some(Layer::RuntimeConsole));
        assert!(!stack.contains(Layer::RuntimeConsole));
        // A task's budget being changed closes before the side panel, and
        // with it.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::TaskBudget);
        assert_eq!(stack.dismiss_top(), Some(Layer::TaskBudget));
        assert!(stack.contains(Layer::FilesPanel));
        stack.open(Layer::TaskBudget);
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::TaskBudget));
        // The mandates page and what opens on it.
        stack.open(Layer::Mandates);
        stack.open(Layer::MandateForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::MandateForm));
        assert!(stack.contains(Layer::Mandates));
        stack.open(Layer::ConfirmDeleteMandate);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDeleteMandate));
        assert!(stack.contains(Layer::Mandates));
        stack.open(Layer::MandateForm);
        assert!(stack.close(Layer::Mandates));
        assert!(!stack.contains(Layer::MandateForm));
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
        // The reason for letting a finding stand is asked inside the
        // finalize panel: Escape takes the reason first, the panel next,
        // and the page stays until a third press.
        stack.open(Layer::Papers);
        stack.open(Layer::PaperFinalize);
        stack.open(Layer::PaperWaiver);
        assert_eq!(stack.dismiss_top(), Some(Layer::PaperWaiver));
        assert!(stack.contains(Layer::PaperFinalize));
        assert_eq!(stack.dismiss_top(), Some(Layer::PaperFinalize));
        assert!(stack.contains(Layer::Papers));
        stack.open(Layer::PaperFinalize);
        stack.open(Layer::PaperWaiver);
        assert!(stack.close(Layer::Papers));
        assert!(!stack.contains(Layer::PaperWaiver) && !stack.contains(Layer::PaperFinalize));
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
        // So is importing conversations.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::ImportConversations);
        assert_eq!(stack.dismiss_top(), Some(Layer::ImportConversations));
        assert!(stack.close(Layer::FilesPanel));
        // The first-run guide is put away with one Escape, and a menu
        // opened above it goes first.
        stack.open(Layer::Onboarding);
        stack.open(Layer::Palette);
        assert_eq!(stack.dismiss_top(), Some(Layer::Palette));
        assert_eq!(stack.dismiss_top(), Some(Layer::Onboarding));
        assert!(!stack.contains(Layer::Onboarding));
        // So is bringing a branch back.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::BranchMerge);
        assert_eq!(stack.dismiss_top(), Some(Layer::BranchMerge));
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

    #[test]
    fn sync_questions_close_before_their_dialog() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::ProjectSettings);
        stack.open(Layer::ConfirmSyncReplace);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmSyncReplace));
        assert!(stack.contains(Layer::ProjectSettings));
        stack.open(Layer::ConfirmSyncReplace);
        assert!(stack.close(Layer::ProjectSettings));
        assert!(!stack.contains(Layer::ConfirmSyncReplace));
        // The join dialog is a layer of its own.
        stack.open(Layer::JoinSyncedProject);
        assert_eq!(stack.dismiss_top(), Some(Layer::JoinSyncedProject));
        assert_eq!(stack.dismiss_top(), None);
        // So is the dialog for a new project.
        stack.open(Layer::NewProject);
        assert_eq!(stack.dismiss_top(), Some(Layer::NewProject));
        assert_eq!(stack.dismiss_top(), None);
    }

    #[test]
    fn the_plugin_address_form_closes_before_settings() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::Settings);
        stack.open(Layer::PluginAddress);
        assert_eq!(stack.dismiss_top(), Some(Layer::PluginAddress));
        assert!(stack.contains(Layer::Settings));
    }

    #[test]
    fn the_github_skill_form_closes_before_settings() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::Settings);
        stack.open(Layer::SkillFromGithub);
        assert_eq!(stack.dismiss_top(), Some(Layer::SkillFromGithub));
        assert!(stack.contains(Layer::Settings));
    }

    #[test]
    fn the_hook_form_closes_before_settings() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::Settings);
        stack.open(Layer::HookForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::HookForm));
        assert!(stack.contains(Layer::Settings));
        stack.open(Layer::HookForm);
        assert!(stack.close(Layer::Settings));
        assert!(!stack.contains(Layer::HookForm));
    }

    #[test]
    fn the_copy_menu_closes_before_the_examples() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::Examples);
        stack.open(Layer::ExampleCopy);
        assert_eq!(stack.dismiss_top(), Some(Layer::ExampleCopy));
        assert!(stack.contains(Layer::Examples));
        // Closing the dialog closes the menu drawn in it.
        stack.open(Layer::ExampleCopy);
        assert!(stack.close(Layer::Examples));
        assert!(!stack.contains(Layer::ExampleCopy));
        assert_eq!(stack.dismiss_top(), None);
    }

    #[test]
    fn exploration_questions_close_before_the_preview() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::ExplorationPreview);
        stack.open(Layer::ConfirmExploration);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmExploration));
        assert!(stack.contains(Layer::ExplorationPreview));
        assert_eq!(stack.dismiss_top(), Some(Layer::ExplorationPreview));
        stack.open(Layer::StartExploration);
        assert_eq!(stack.dismiss_top(), Some(Layer::StartExploration));
        assert_eq!(stack.dismiss_top(), None);
    }
}
