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
    /// Marking regions on a previewed picture: a drag draws a region
    /// instead of moving the picture, and a click on a note's number
    /// removes it.
    PictureSelect,
    /// A region just marked on the picture, with the field for its note and
    /// the actions that put it on the message.
    PictureRegion,
    /// Question over the preview when an edited script changed on disk
    /// after the edit began: keep editing, reload, or overwrite.
    FileChangedOnDisk,
    /// Question before the window closes over edits that are not saved.
    ConfirmCloseUnsaved,
    /// The side panel's tabs that can be added back, under its tab row.
    PanelTabMenu,
    /// Question under a run's card before its workspace is deleted.
    ConfirmCleanRun,
    /// Where the project keeps things on one SSH host, in the
    /// Environments tab.
    HostStorageForm,
    /// What a finished run left on its SSH host: a dialog over everything.
    RunReview,
    /// The projects to hide while recording: a dialog over everything.
    PrivacyMode,
    /// Question in that dialog before files there are deleted.
    ConfirmRunFilesDelete,
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
    /// The open conversation's actions, under the More button of its
    /// header. Group and Move open beside it.
    ConversationMenu,
    /// The research journey of the open project: a page of its own.
    Journey,
    /// The form on the journey page for a record of the researcher's own.
    JournalForm,
    /// A day's recap being changed, on the journey page.
    RecapForm,
    /// The details of one relation in the journey's relation graph.
    JourneyEdge,
    /// A run's record, as a dialog over the journey page.
    JourneyRun,
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
    /// The choice of how to compact the open conversation: a dialog over
    /// it, or over What the model is sent when opened from there.
    CompactContext,
    /// Handing the open conversation off to a new one: a dialog over it,
    /// or over What the model is sent when opened from there.
    Handoff,
    /// The first of a range of the open conversation's turns is chosen
    /// (Quote / reuse): a bar above the composer until the last is.
    ReuseRange,
    /// The chosen turns, to quote in another conversation or distill into
    /// an experience: a dialog over the conversation, with the range under
    /// it.
    SessionReuse,
    /// The research archive of the open conversation: a dialog over it.
    ResearchArchive,
    /// Question over that dialog before the changes to its draft are
    /// given up.
    ConfirmDiscardArchive,
    /// The messages of the open conversation to put into its shared page:
    /// a dialog over it.
    ShareMessages,
    /// Bringing the open branch back into its source: a dialog over it.
    BranchMerge,
    /// Conversations from Codex or Claude Code to import: a dialog.
    ImportConversations,
    /// The WebView desktop's projects to bring over: a dialog.
    WebViewImport,
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
    /// Question, as a dialog, before every note of the project's memory is
    /// deleted (the Memory tab's Clear all).
    ConfirmClearMemory,
    /// The Favorites page, shown in place of the rest of the window.
    Favorites,
    /// Question on the Favorites page before a saved item is removed.
    ConfirmRemoveFavorite,
    /// A saved item's code being edited into a new version, on the
    /// Favorites page.
    FavoriteCode,
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
    /// The cards for conversations in other projects that finished or
    /// failed, in the window's corner over the page. Escape closes the
    /// newest card once nothing else is open.
    SessionNotices,
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
            | Layer::PanelTabMenu
            | Layer::ConfirmCleanRun
            | Layer::HostStorageForm
            | Layer::FileName
            | Layer::ConfirmTrashFile
            | Layer::MemoEditor
            | Layer::ConfirmDeleteMemo
            | Layer::MemoryEditor
            | Layer::ConfirmDeleteMemory
            | Layer::ConfirmClearMemory
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
            Layer::ConfirmRemoveFavorite | Layer::FavoriteCode => Some(Layer::Favorites),
            Layer::ConfirmSyncReplace => Some(Layer::ProjectSettings),
            Layer::JournalForm | Layer::RecapForm | Layer::JourneyEdge | Layer::JourneyRun => {
                Some(Layer::Journey)
            }
            Layer::ScheduleForm | Layer::ConfirmDeleteSchedule => Some(Layer::Schedules),
            Layer::MandateForm | Layer::ConfirmDeleteMandate => Some(Layer::Mandates),
            Layer::PaperForm
            | Layer::PaperItemForm
            | Layer::PaperSources
            | Layer::ConfirmRemovePaper
            | Layer::PaperFinalize => Some(Layer::Papers),
            Layer::PaperWaiver => Some(Layer::PaperFinalize),
            Layer::BatchGroupMenu | Layer::ConfirmBatchDelete => Some(Layer::SelectMode),
            Layer::GroupMenu | Layer::TransferMenu => Some(Layer::ConversationMenu),
            Layer::ExampleCopy => Some(Layer::Examples),
            Layer::ConfirmRunFilesDelete => Some(Layer::RunReview),
            Layer::FileChangedOnDisk | Layer::PictureSelect => Some(Layer::FilePreview),
            Layer::PictureRegion => Some(Layer::PictureSelect),
            Layer::SessionReuse => Some(Layer::ReuseRange),
            Layer::ConfirmDiscardArchive => Some(Layer::ResearchArchive),
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

    /// Opens `layer` under every surface already open, so Escape reaches it
    /// only after them. Already open, it keeps its place.
    pub fn open_beneath(&mut self, layer: Layer) {
        if !self.0.contains(&layer) {
            self.0.insert(0, layer);
        }
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
    /// The projects hidden while recording.
    Privacy,
}

/// `primary` is Cmd on macOS and Ctrl elsewhere. Shift makes a chord of its
/// own; Alt or the other platform key leave the chord to whoever else
/// wants it.
pub fn shortcut(key: &str, primary: bool, shift: bool, others: bool) -> Option<Shortcut> {
    if !primary || others {
        return None;
    }
    if shift {
        return match key {
            "f" => Some(Shortcut::SearchHistory),
            "h" => Some(Shortcut::Privacy),
            _ => None,
        };
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
    fn the_share_dialog_closes_alone_and_outlives_the_side_panel() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::ShareMessages);
        assert_eq!(stack.dismiss_top(), Some(Layer::ShareMessages));
        assert!(stack.contains(Layer::FilesPanel));
        // It is the window's, not the panel's.
        stack.open(Layer::ShareMessages);
        assert!(stack.close(Layer::FilesPanel));
        assert!(stack.contains(Layer::ShareMessages));
    }

    #[test]
    fn the_question_about_a_changed_file_goes_first_and_with_its_preview() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::FilePreview);
        stack.open(Layer::FileChangedOnDisk);
        // One press closes the question; the edit stays open under it.
        assert_eq!(stack.dismiss_top(), Some(Layer::FileChangedOnDisk));
        assert!(stack.contains(Layer::FilePreview));
        // Leaving the preview takes a question about it along.
        stack.open(Layer::FileChangedOnDisk);
        assert!(stack.close(Layer::FilePreview));
        assert!(!stack.contains(Layer::FileChangedOnDisk));
        assert!(stack.contains(Layer::FilesPanel));
        // The question before closing the window stands over everything and
        // goes on its own.
        stack.open(Layer::FilePreview);
        stack.open(Layer::ConfirmCloseUnsaved);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmCloseUnsaved));
        assert!(stack.contains(Layer::FilePreview));
    }

    #[test]
    fn a_marked_region_goes_first_then_marking_then_the_picture() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::FilePreview);
        stack.open(Layer::PictureSelect);
        stack.open(Layer::PictureRegion);
        // Escape, pressed at once, gives up the region and its note; the
        // picture is still being marked, and its preview is open.
        assert_eq!(stack.dismiss_top(), Some(Layer::PictureRegion));
        assert!(stack.contains(Layer::PictureSelect) && stack.contains(Layer::FilePreview));
        assert_eq!(stack.dismiss_top(), Some(Layer::PictureSelect));
        assert!(stack.contains(Layer::FilePreview));
        // Leaving marking takes a region waiting for its note along, and
        // leaving the picture takes both.
        stack.open(Layer::PictureSelect);
        stack.open(Layer::PictureRegion);
        assert!(stack.close(Layer::PictureSelect));
        assert!(!stack.contains(Layer::PictureRegion));
        stack.open(Layer::PictureSelect);
        stack.open(Layer::PictureRegion);
        assert!(stack.close(Layer::FilePreview));
        assert!(!stack.contains(Layer::PictureSelect) && !stack.contains(Layer::PictureRegion));
        assert!(stack.contains(Layer::FilesPanel));
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

        // The tabs to add back go with the panel, and alone on Escape.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::PanelTabMenu);
        assert_eq!(stack.dismiss_top(), Some(Layer::PanelTabMenu));
        assert!(stack.contains(Layer::FilesPanel));
        stack.open(Layer::PanelTabMenu);
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::PanelTabMenu));

        // The question in the run review closes before the review.
        stack.open(Layer::RunReview);
        stack.open(Layer::ConfirmRunFilesDelete);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmRunFilesDelete));
        assert_eq!(stack.dismiss_top(), Some(Layer::RunReview));
        stack.open(Layer::RunReview);
        stack.open(Layer::ConfirmRunFilesDelete);
        assert!(stack.close(Layer::RunReview));
        assert!(!stack.contains(Layer::ConfirmRunFilesDelete));

        // So does a host's storage form.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::HostStorageForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::HostStorageForm));
        assert!(stack.contains(Layer::FilesPanel));
        stack.close(Layer::FilesPanel);

        // So does the question before a run's workspace is cleaned.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::ConfirmCleanRun);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmCleanRun));
        assert!(stack.contains(Layer::FilesPanel));
        stack.open(Layer::ConfirmCleanRun);
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::ConfirmCleanRun));
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
    fn notices_from_other_projects_wait_under_what_is_already_open() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::Settings);
        stack.open(Layer::McpForm);
        // A card arrives while a form is open: the form closes first.
        stack.open_beneath(Layer::SessionNotices);
        assert_eq!(stack.dismiss_top(), Some(Layer::McpForm));
        // Another card does not move the cards up.
        stack.open_beneath(Layer::SessionNotices);
        assert_eq!(stack.dismiss_top(), Some(Layer::Settings));
        assert_eq!(stack.dismiss_top(), Some(Layer::SessionNotices));
        assert_eq!(stack.dismiss_top(), None);
        // Alone, a press reaches the cards at once; what opens after them
        // is closed before them.
        stack.open_beneath(Layer::SessionNotices);
        stack.open(Layer::Palette);
        assert_eq!(stack.dismiss_top(), Some(Layer::Palette));
        assert!(stack.contains(Layer::SessionNotices));
        assert!(stack.close(Layer::SessionNotices));
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
        // With Shift, H is privacy mode, as in the WebView.
        assert_eq!(shortcut("h", true, true, false), Some(Shortcut::Privacy));
        assert_eq!(shortcut("h", true, false, false), None);
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
    fn the_conversation_menu_closes_after_what_it_opened_beside_it() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::ConversationMenu);
        stack.switch(Layer::GroupMenu, &Layer::CONVERSATION_BAR);
        // Escape, pressed right after opening, closes only the topmost.
        assert_eq!(stack.dismiss_top(), Some(Layer::GroupMenu));
        assert!(stack.contains(Layer::ConversationMenu));
        // Closing the menu takes what it opened with it.
        stack.switch(Layer::TransferMenu, &Layer::CONVERSATION_BAR);
        assert!(stack.close(Layer::ConversationMenu));
        assert!(!stack.contains(Layer::TransferMenu));
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
        // The question before Clear all is the one Escape closes first, and
        // it goes with the panel its tab is in.
        stack.open(Layer::ConfirmClearMemory);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmClearMemory));
        assert!(stack.contains(Layer::FilesPanel) && stack.contains(Layer::MemoryEditor));
        stack.open(Layer::ConfirmClearMemory);
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::MemoEditor) && !stack.contains(Layer::MemoryEditor));
        assert!(!stack.contains(Layer::ConfirmClearMemory));
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
        // An edit of saved code is inside the page: Escape gives it up and
        // leaves the page, and leaving the page takes it along.
        stack.open(Layer::Favorites);
        stack.open(Layer::FavoriteCode);
        assert_eq!(stack.dismiss_top(), Some(Layer::FavoriteCode));
        assert!(stack.contains(Layer::Favorites));
        stack.open(Layer::FavoriteCode);
        assert!(stack.close(Layer::Favorites));
        assert!(!stack.contains(Layer::FavoriteCode));
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
    fn the_compact_dialog_closes_first_and_leaves_what_the_model_is_sent() {
        let mut stack = EscapeStack::default();
        // From /compact: one Escape and it is gone.
        stack.open(Layer::CompactContext);
        assert_eq!(stack.dismiss_top(), Some(Layer::CompactContext));
        assert_eq!(stack.dismiss_top(), None);
        // From What the model is sent: Escape leaves the choice first and
        // the dialog under it stays open; closing that one does not take
        // the choice along, which stands on its own.
        stack.open(Layer::ModelView);
        stack.open(Layer::CompactContext);
        assert_eq!(stack.dismiss_top(), Some(Layer::CompactContext));
        assert!(stack.contains(Layer::ModelView));
        stack.open(Layer::CompactContext);
        assert!(stack.close(Layer::ModelView));
        assert!(stack.contains(Layer::CompactContext));
    }

    #[test]
    fn the_handoff_dialog_closes_first_and_leaves_what_the_model_is_sent() {
        let mut stack = EscapeStack::default();
        // From the command palette: one Escape and it is gone.
        stack.open(Layer::Handoff);
        assert_eq!(stack.dismiss_top(), Some(Layer::Handoff));
        assert_eq!(stack.dismiss_top(), None);
        // From What the model is sent: the note goes first and the dialog
        // under it stays open, as the WebView's usage panel does.
        stack.open(Layer::ModelView);
        stack.open(Layer::Handoff);
        assert_eq!(stack.dismiss_top(), Some(Layer::Handoff));
        assert!(stack.contains(Layer::ModelView));
        assert_eq!(stack.dismiss_top(), Some(Layer::ModelView));
        assert_eq!(stack.dismiss_top(), None);
    }

    #[test]
    fn the_reuse_dialog_closes_first_and_leaves_the_range_chosen() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::ReuseRange);
        stack.open(Layer::SessionReuse);
        // One Escape closes the dialog; the range stays, as in the WebView.
        assert_eq!(stack.dismiss_top(), Some(Layer::SessionReuse));
        assert!(stack.contains(Layer::ReuseRange));
        // The next gives up the range.
        assert_eq!(stack.dismiss_top(), Some(Layer::ReuseRange));
        assert_eq!(stack.dismiss_top(), None);
        // Giving up the range, or leaving the conversation, takes the
        // dialog along.
        stack.open(Layer::ReuseRange);
        stack.open(Layer::SessionReuse);
        assert!(stack.close(Layer::ReuseRange));
        assert!(!stack.contains(Layer::SessionReuse));
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
        // So is a recap being changed.
        stack.open(Layer::Journey);
        stack.open(Layer::RecapForm);
        assert_eq!(stack.dismiss_top(), Some(Layer::RecapForm));
        assert!(stack.contains(Layer::Journey));
        // A relation's details and a run's record are inside the page:
        // one press closes the topmost, and the page stays.
        stack.open(Layer::JourneyEdge);
        stack.open(Layer::JourneyRun);
        assert_eq!(stack.dismiss_top(), Some(Layer::JourneyRun));
        assert_eq!(stack.dismiss_top(), Some(Layer::JourneyEdge));
        assert!(stack.contains(Layer::Journey));
        // Leaving the page takes both along.
        stack.open(Layer::JourneyEdge);
        stack.open(Layer::JourneyRun);
        assert!(stack.close(Layer::Journey));
        assert!(!stack.contains(Layer::JourneyEdge) && !stack.contains(Layer::JourneyRun));
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
        // The question before its draft's changes are given up goes first,
        // and leaves the draft open under it.
        stack.open(Layer::ResearchArchive);
        stack.open(Layer::ConfirmDiscardArchive);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDiscardArchive));
        assert!(stack.contains(Layer::ResearchArchive));
        // Closing the dialog closes the question with it.
        stack.open(Layer::ConfirmDiscardArchive);
        assert!(stack.close(Layer::ResearchArchive));
        assert!(!stack.contains(Layer::ConfirmDiscardArchive));
        assert!(stack.close(Layer::FilesPanel));
        // So is importing conversations.
        stack.open(Layer::FilesPanel);
        stack.open(Layer::ImportConversations);
        assert_eq!(stack.dismiss_top(), Some(Layer::ImportConversations));
        assert!(stack.close(Layer::FilesPanel));
        // And bringing the WebView's projects over, opened from the palette.
        stack.open(Layer::Palette);
        stack.close(Layer::Palette);
        stack.open(Layer::WebViewImport);
        assert_eq!(stack.dismiss_top(), Some(Layer::WebViewImport));
        assert_eq!(stack.dismiss_top(), None);
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
