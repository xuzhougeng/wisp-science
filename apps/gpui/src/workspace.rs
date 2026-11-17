mod agent_cards_view;
mod agents_view;
mod archive_view;
mod aside_view;
mod assistant_view;
mod browser_prompts_view;
mod calendar_view;
mod capabilities_view;
mod centre_view;
mod compact_view;
mod conversation_actions;
mod demos_view;
mod explore_view;
mod favorites_view;
mod file_editor_view;
mod files_panel;
mod find_view;
mod follow_ups_view;
mod generated_view;
mod handoff_view;
mod home_view;
mod hosts_view;
mod import_view;
mod inline_view;
mod journey_graph_view;
mod journey_view;
mod mandates_view;
mod memory_view;
mod memos_view;
mod merge_view;
mod model_view_dialog;
mod notebook_view;
mod notices_view;
mod onboarding_view;
mod organise_view;
mod palette_view;
mod panel_tabs;
mod papers_view;
mod pet_view;
mod picture_view;
mod privacy_view;
mod project_settings;
mod project_sync_view;
mod queue_view;
mod remote_files_view;
mod reuse_view;
mod review_view;
mod run_cards_view;
mod run_review_view;
mod runtime_console_view;
mod schedules_view;
mod settings_acp;
mod settings_browser;
mod settings_channels;
mod settings_data;
mod settings_decision;
mod settings_environments;
mod settings_experts;
mod settings_hooks;
mod settings_lab;
mod settings_mcp;
mod settings_network;
mod settings_plugins;
mod settings_quick;
mod settings_remote;
mod settings_sections;
mod settings_skill_store;
mod settings_subscriptions;
mod settings_view;
mod settings_workflows;
mod share_view;
mod shell;
mod slash_view;
mod stream_recovery_view;
mod surfaces;
mod tabs_view;
mod terminal_view;
mod trajectory_view;
mod transfer_view;
mod transfers_view;
mod webview_import_view;

use crate::i18n::tr;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

actions!(composer, [SendMessage]);
pub use file_editor_view::file_editor_bindings;
pub use surfaces::window_titlebar;

/// The composer's own keys, bound after the input component's so they win
/// where both apply: with the Return choice, Return sends and Shift+Return
/// starts a new line. Elsewhere, and with the other choice, the input keeps
/// Return for a new line and Cmd/Ctrl+Return sends.
pub fn composer_bindings() -> Vec<KeyBinding> {
    let inside = format!("{} > Input", crate::settings::SEND_ON_ENTER);
    vec![
        KeyBinding::new("enter", SendMessage, Some(&inside)),
        KeyBinding::new(
            "shift-enter",
            gpui_component::input::Enter { secondary: false },
            Some(&inside),
        ),
    ]
}

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, StyledExt,
    input::{InputEvent, InputState},
};
use serde_json::{Value, json};
use wisp_dto::{ProjectSummary, SessionSearchInfo, native_conversations::Item};

use wisp_dto::native_conversations::SendRequest;
use wisp_server::desktop::{
    ArtifactRow, ConversationOptions, ConversationStatus, Favorite, Folder, LibraryItem,
    LibraryItemSummary, MemoryView, ProjectMemo, TurnRef,
};

use crate::{
    archive,
    backend::{Backend, receive},
    compose::{self, Chip, Pending},
    controls,
    files::FilesState,
    home::{self, Screen},
    layers::{EscapeStack, Layer, Shortcut, is_escape, shortcut},
    organise::{self, Arrangement, Picks},
    palette::{self, PaletteState},
    panel::{self, PanelTab},
    present::{self, Tone},
    state::WorkspaceState,
};

pub struct Workspace {
    backend: Backend,
    state: WorkspaceState,
    /// Home page or one project, under whatever layers are open.
    screen: Screen,
    projects: Vec<ProjectSummary>,
    /// Newest conversations in any project, for the home page.
    recent: Vec<SessionSearchInfo>,
    /// Project waiting for the remove confirmation on the home page.
    removing: Option<String>,
    /// The second, stronger question: delete the project's folder too.
    removing_folder: Option<home::FolderQuestion>,
    delete_name_input: Entity<InputState>,
    /// Tool rows whose input and output are unfolded.
    open_tools: HashSet<String>,
    /// Files and references attached to the message being written, per
    /// conversation, like the drafts.
    chips: HashMap<String, Pending>,
    /// Per conversation, the messages sent from this window with
    /// references and what they carried, to show on them.
    sent_references: HashMap<String, Vec<(String, Vec<crate::compose::Chip>)>>,
    /// The composer's text before its latest change, to tell one character
    /// typed from the rest.
    composer_before: String,
    /// What follows the `/` the composer ends in, while the `/` menu
    /// follows it.
    slash_typed: Option<String>,
    /// Escape shut the `/` menu: it stays shut until that word is gone.
    slash_quiet: bool,
    /// The skills the `/` menu offers for what was typed.
    slash_skills: Vec<wisp_dto::native_conversations::ReferenceOption>,
    /// Options, usage and remembered approvals of the open conversation.
    status: Option<ConversationStatus>,
    /// Counts snapshot polls, so the status is refreshed about once a second.
    status_tick: u8,
    /// The artifact whose versions are shown, and the versions read.
    artifact_open: Option<String>,
    artifact_versions: HashMap<String, Vec<wisp_server::desktop::ArtifactVersionRow>>,
    /// The Provenance tab's calls a click opened or closed, by their key.
    provenance_open: HashMap<String, bool>,
    /// What each saved turn of the open conversation used, read when it
    /// opens and when a turn ends.
    turn_tokens: Vec<wisp_server::desktop::TurnTokens>,
    /// Transcript row whose text was just copied, for a moment of feedback.
    copied: Option<usize>,
    /// The message whose More menu is open (`Layer::MessageMenu`).
    message_menu: Option<usize>,
    palette: PaletteState,
    palette_input: Entity<InputState>,
    /// The overview of what the open project can draw on, once read, and
    /// why it could not be.
    capabilities: Option<wisp_server::desktop::Capabilities>,
    capabilities_error: Option<String>,
    /// Where the research archive dialog stands.
    archive: Option<archive_view::ArchiveView>,
    archive_error: Option<String>,
    /// Bringing the open branch back, and its summary's field.
    merge: Option<merge_view::MergeView>,
    merge_error: Option<String>,
    merge_summary: Entity<InputState>,
    /// The import dialog.
    imports: import_view::Imports,
    webview_imports: webview_import_view::WebViewImports,
    /// Title, report, and the name and text of the script shown.
    archive_fields: [Entity<InputState>; 4],
    model_view: Option<wisp_server::desktop::ModelView>,
    model_view_error: Option<String>,
    /// The rows opened to show their whole text.
    model_view_open: HashSet<usize>,
    /// The kind of the latest request opened to show what it holds; one
    /// at a time, as in the WebView's context usage.
    model_view_kind: Option<crate::model_view::Sent>,
    /// The kind of compaction the compact dialog has chosen.
    compact_kind: crate::compaction::Kind,
    /// A compaction asked for there is running.
    compacting: bool,
    /// Why the compaction asked for there failed.
    compact_error: Option<String>,
    /// The conversation whose refused turn goes on once the compaction
    /// asked for there is done (Compact and continue).
    compact_resume: Option<String>,
    /// The turns refused as too long whose bar the user paused on.
    paused_failures: HashSet<String>,
    /// Handing the open conversation off to a new one, and why that
    /// failed.
    handoff: Option<crate::handoff::Dialog>,
    handoff_error: Option<String>,
    /// The handoff note's instruction and the note.
    handoff_fields: [Entity<InputState>; 2],
    /// The first turn of a range of the open conversation being chosen to
    /// quote or distill (Quote / reuse).
    reuse_selection: Option<crate::reuse::Selection>,
    /// The chosen turns are being read for the dialog.
    reuse_loading: bool,
    /// The dialog over the chosen turns, and why its last step failed.
    reuse: Option<crate::reuse::Dialog>,
    reuse_error: Option<String>,
    /// Its focus for the experience, the experience draft, and the search
    /// for a destination.
    reuse_fields: [Entity<InputState>; 3],
    /// The turn the review dialog is about; `None` is the whole conversation.
    review_turn: Option<wisp_server::desktop::TurnRef>,
    /// The review the dialog is showing or waiting for.
    review: Option<crate::review::Asked>,
    trajectory: Option<wisp_dto::TrajectorySnapshotDto>,
    trajectory_error: Option<String>,
    /// The steps opened to show their whole text: (turn, step).
    trajectory_open: HashSet<(usize, usize)>,
    /// The calendar across projects: the month shown, every project's
    /// records once read, the counter that drops late answers, the day
    /// that is chosen, and the projects left out.
    calendar_month: Option<crate::journey::Month>,
    calendar: Option<Vec<wisp_dto::ResearchCalendarProject>>,
    calendar_request: u64,
    calendar_day: Option<u32>,
    calendar_hidden: HashSet<String>,
    /// The month of the open project's history that is shown, its records
    /// once read, the counter that drops late answers, the filter field,
    /// and the form for a record of the researcher's own (title, details)
    /// with the kind chosen for it.
    journey_month: Option<crate::journey::Month>,
    journey: Option<wisp_dto::ResearchJourney>,
    journey_request: u64,
    journey_search: Entity<InputState>,
    /// The local day whose recap is being drafted, by its midnight.
    recap_drafting: Option<i64>,
    /// The journey's Relationships view: shown in place of the days, the
    /// project's research graph once read, drawn as a graph rather than
    /// listed, and the relation whose details are open.
    journey_graph_shown: bool,
    journey_graph: Option<wisp_dto::ResearchGraph>,
    journey_graph_drawn: bool,
    journey_edge: Option<wisp_dto::ResearchEdge>,
    /// The output whose source is shown beside the days, and that source
    /// once read.
    journey_output: Option<wisp_dto::ResearchJourneyEntry>,
    journey_source: Option<Result<wisp_server::desktop::JourneySource, String>>,
    /// The run whose record is open over the page, and the record once read.
    journey_run: Option<(
        String,
        Option<Result<wisp_server::desktop::JourneyRun, String>>,
    )>,
    /// The research assistant's plan for today.
    assistant_plan: Vec<wisp_dto::ResearchAssistantPlanItem>,
    /// The weekly habit tidy, on the Memory tab.
    memory_tidy: Option<wisp_dto::MemoryTidyAutomation>,
    /// The interpreter the console is on, what it holds, and what was run.
    console_runtime: Option<String>,
    console_objects: Option<wisp_server::desktop::RuntimeObjectList>,
    console_entries: Vec<crate::console::Entry>,
    console_input: Entity<InputState>,
    console_busy: bool,
    /// Kept at the newest run, so what was just run is what shows.
    console_scroll: ScrollHandle,
    /// The research mandates page.
    mandates: Option<Vec<wisp_dto::MandateOverview>>,
    /// The mandate the form changes; none for a new one.
    mandate_editing: Option<String>,
    mandate_project: Option<String>,
    mandate_interval: i64,
    mandate_report_interval: i64,
    mandate_review: bool,
    /// Name, goal, KPI, and the reply to a mandate's request.
    mandate_fields: [Entity<InputState>; 4],
    mandate_deleting: Option<String>,
    /// The mandate whose request is being answered.
    mandate_replying: Option<String>,
    /// The daily recap run, on the Scheduled tasks page.
    daily_recap: Option<wisp_dto::DailyRecapAutomation>,
    recap_time: Entity<InputState>,
    /// Every project's scheduled tasks; `None` until the host answered.
    schedules: Option<Vec<wisp_server::desktop::ScheduleRow>>,
    /// Name, prompt, hours, time of day.
    schedule_fields: [Entity<InputState>; 4],
    schedule_project: Option<String>,
    schedule_repeat: crate::schedules::Repeat,
    /// Monday is 0.
    schedule_weekday: u32,
    schedule_deleting: Option<String>,
    /// What the last change said.
    schedule_note: Option<String>,
    /// The open project's papers, and the one the page shows.
    papers: Vec<wisp_server::desktop::PaperRow>,
    paper: Option<wisp_server::desktop::Paper>,
    paper_request: u64,
    /// The paper form renames the shown paper instead of adding one.
    paper_renaming: bool,
    /// Title, description, first version.
    paper_fields: [Entity<InputState>; 3],
    /// Title, note.
    paper_item_fields: [Entity<InputState>; 2],
    paper_item_kind: wisp_server::desktop::PaperItemKind,
    /// The item the one in the form sits under; `None` at the top level.
    paper_item_parent: Option<String>,
    /// The item in the form; empty is a new one.
    paper_item_editing: Option<String>,
    /// The item evidence is being chosen for.
    paper_source_item: Option<String>,
    paper_source_kind: wisp_server::desktop::PaperSourceKind,
    /// Search, why it was chosen.
    paper_source_fields: [Entity<InputState>; 2],
    /// Whether what is chosen is meant for the paper, and who may see it.
    paper_source_selection: wisp_server::desktop::PaperSelection,
    paper_source_audience: wisp_server::desktop::PaperAudience,
    /// The exact anchor being typed in the advanced dialog: its kind as
    /// the WebView names it, then the session, message, byte range, tool
    /// call and exact id.
    paper_anchor_kind: &'static str,
    paper_anchor_fields: [Entity<InputState>; 6],
    /// The evidence whose source and environment details are open.
    paper_sources_open: std::collections::HashSet<String>,
    paper_choices: Vec<wisp_server::desktop::PaperSourceChoice>,
    paper_choices_more: bool,
    /// The conversation message a passage is being chosen from, and its
    /// text, read-only, to select the passage in.
    paper_passage: Option<wisp_server::desktop::PaperSourceChoice>,
    paper_passage_field: Entity<InputState>,
    /// The run of the shown finalized version being rerun clean, and
    /// whether its capsule is being written.
    paper_rerunning: Option<String>,
    paper_capsule_saving: bool,
    /// What finalizing the shown version is checked against, what the
    /// last check found, and whether one is under way.
    paper_policy: wisp_server::desktop::PaperPolicy,
    paper_check: Option<wisp_server::desktop::PaperCheck>,
    paper_checking: bool,
    /// The finding a reason is being given for, and the reason.
    paper_waiving: Option<String>,
    paper_waiver_field: Entity<InputState>,
    journal_fields: [Entity<InputState>; 2],
    /// The recap being changed, by id.
    recap_editing: Option<String>,
    /// Its headline, then what was done, found, in the way and next, one
    /// item a line.
    recap_fields: [Entity<InputState>; 5],
    journal_category: &'static str,
    /// What the Files panel's name field is for, and the field; the entry
    /// the trash question is about; why the last change was refused; and
    /// where the last trashed entry went.
    file_naming: Option<crate::files::Naming>,
    file_name_input: Entity<InputState>,
    /// The Files panel's field for finding a file by name.
    file_search: Entity<InputState>,
    file_trashing: Option<String>,
    file_problem: Option<String>,
    file_notice: Option<String>,
    /// A table file is shown as its text instead.
    file_as_text: bool,
    /// What is being looked for in the open conversation, and its field.
    find: crate::find::Find,
    find_input: Entity<InputState>,
    aside_input: Entity<InputState>,
    /// What should change in a plan the model asks to start on.
    plan_feedback: Entity<InputState>,
    /// What was asked aside, by conversation. Kept until the app closes.
    asides: HashMap<String, crate::aside::Thread>,
    /// The run the next side question is about, with its title, when it
    /// was asked from a run card.
    aside_run: Option<(String, String)>,
    /// Passages selected in project files, quoted into the next side
    /// question, as the WebView's side chat holds its quotes.
    aside_quotes: Vec<crate::aside::Quote>,
    palette_scroll: ScrollHandle,
    sessions: Vec<SessionSearchInfo>,
    /// Conversations in any project that are waiting for the user.
    inbox: Vec<SessionSearchInfo>,
    /// Rows seen in any list, so the open conversation keeps its title and
    /// state while a search or the archive view does not include it.
    known: HashMap<String, SessionSearchInfo>,
    /// The conversations opened in this window, as tabs.
    tabs: crate::tabs::Tabs,
    /// The project files each conversation has open in the centre, as tabs
    /// after its own, and the split that puts it beside the one shown.
    centre: crate::centre::Centre,
    /// The document tab whose menu is open, by its path.
    centre_menu: Option<String>,
    /// How wide the split centre was when last drawn, for its widths.
    centre_width: std::rc::Rc<std::cell::Cell<Option<f32>>>,
    /// The side panel's width as last drawn, for how many tabs its strip
    /// shows.
    panel_width: std::rc::Rc<std::cell::Cell<Option<f32>>>,
    /// The SSH host folder the Files tab is looking at, instead of the
    /// project's own files.
    remote: Option<crate::files::RemoteBrowse>,
    /// Whole logs that were asked for, by run: the log, or why not.
    run_logs: HashMap<String, Result<wisp_server::desktop::RunLog, String>>,
    /// The method search whose card is open in the Runs tab.
    open_search: Option<String>,
    /// What its card last read.
    search_details: Option<wisp_dto::MethodSearchRunDetails>,
    /// A read or control of it is under way.
    search_busy: bool,
    /// Why its last read or control did not happen.
    search_problem: Option<String>,
    /// One of its promoted files being read: the version, and its text or
    /// why not.
    search_output: Option<(String, Result<String, String>)>,
    /// Where the previewed file came from, once asked: its path, and the
    /// call that wrote it, none known, or why it could not be looked up.
    file_origin: Option<(
        String,
        Result<Option<wisp_server::desktop::FileOrigin>, String>,
    )>,
    pinned: Vec<String>,
    show_archived: bool,
    /// Only the latest list request may replace the rows on screen.
    list_request: u64,
    /// How many pages of the conversation list are loaded, and whether
    /// older ones were left out.
    paging: organise::Paging,
    /// Cards for conversations in other projects that finished or failed
    /// while this window shows something else.
    notices: crate::notices::Notices,
    /// Takes a finished conversation's card away once it has lingered.
    notice_timer: Option<Task<()>>,
    /// How the conversation list is sorted and sectioned.
    arrangement: Arrangement,
    /// The project's groups, in the user's order.
    folders: Vec<Folder>,
    /// Conversation id to the id of its group.
    grouped: HashMap<String, String>,
    /// Branch id to the id of the conversation it was branched from.
    branched: HashMap<String, String>,
    /// Subagent conversation id to the conversation that started it.
    dispatched: HashMap<String, String>,
    /// The open conversation's timer, as the host last gave it; drawn
    /// only while that conversation is open.
    timer: Option<wisp_server::desktop::ScheduleRow>,
    /// A change to the timer is on its way to the host.
    timer_busy: bool,
    /// The message the rewind question is about.
    rewind_turn: Option<TurnRef>,
    /// Keys of the sections that are folded.
    collapsed: HashSet<String>,
    /// Conversations picked in select mode.
    picks: Picks,
    /// The group whose actions, name field or delete question is showing.
    /// `None` while the name field is open means a new group.
    folder_target: Option<String>,
    folder_name: Entity<InputState>,
    /// The transfer menu copies instead of moving.
    transfer_copy: bool,
    /// Moving takes the conversation's artifacts along; and the preview of
    /// that for the project chosen, with its name, waiting to be confirmed.
    transfer_artifacts: bool,
    transfer_preview: Option<(String, String, wisp_dto::SessionArtifactPreview)>,
    /// The messages being chosen for the open conversation's shared page,
    /// while that dialog is open.
    sharing: Option<crate::share::Sharing>,
    /// Something that went as asked and is worth a line.
    notice: Option<String>,
    /// The project's memos; `None` until they were read.
    memos: Option<Vec<ProjectMemo>>,
    /// Only the latest memo request may fill the list.
    memos_request: u64,
    /// The memo being edited or asked about; `None` in the editor means a
    /// new one.
    memo_target: Option<String>,
    memo_title: Entity<InputState>,
    memo_body: Entity<InputState>,
    /// The Memos tab's search field: it narrows the list to the memos whose
    /// title or text holds what is typed.
    memo_search: Entity<InputState>,
    /// What is remembered, as the Memory tab shows it; `None` until read.
    memory: Option<MemoryView>,
    /// Only the latest memory request may fill the tab.
    memory_request: u64,
    /// What the memory editor is writing, or the delete question is about.
    memory_target: Option<crate::memory::Target>,
    /// The model is drafting a memory, and where it said to keep the last draft.
    memory_suggesting: bool,
    memory_suggested: Option<String>,
    memory_text: Entity<InputState>,
    /// Which palette the window wears.
    appearance: crate::theme::Appearance,
    /// Palettes and text sizes and faces, over light or dark.
    look: crate::theme::Look,
    /// Text face and code face being typed on the General page.
    font_fields: [Entity<InputState>; 2],
    language: crate::i18n::Language,
    /// How the user is told that a conversation needs them.
    notify: crate::notify::Choice,
    /// Which of the system's sounds is played.
    notify_tone: &'static str,
    /// The check for a newer version, once one was asked for.
    update: Option<crate::settings::Update>,
    /// Look for a newer version when the app starts.
    update_at_start: bool,
    /// A newer version the check at start found, until it is put away.
    update_banner: Option<wisp_server::desktop::UpdateCheck>,
    /// What the app opens on.
    launch: crate::settings::Launch,
    /// Which key sends the composer's text.
    send_key: crate::settings::SendKey,
    /// Whether the project's sidebar is put away, for more room.
    sidebar_hidden: bool,
    /// The Settings page that is showing.
    settings_section: crate::settings::Section,
    /// How turns run and new conversations start; `None` until read.
    turn_defaults: Option<wisp_server::desktop::TurnDefaults>,
    /// When a turn whose tools failed often has a memory drafted.
    failure_analysis: Option<wisp_server::desktop::AutoFailureAnalysisSettings>,
    /// The model profile reviews are made with; empty is the conversation's.
    review_model: String,
    /// The model profile sub-agents work on; empty is the conversation's.
    subagent_model: String,
    /// The model profile that describes pictures for a model that cannot
    /// see; empty is the first saved one that can.
    picture_model: String,
    steps_input: Entity<InputState>,
    /// Where the local environment's executables are, each with the field
    /// its path is typed into.
    tool_paths: Vec<(wisp_server::desktop::ToolPath, Entity<InputState>)>,
    /// The paths were saved, since they were last shown.
    tool_paths_saved: bool,
    /// Tools that are always allowed, project by project.
    grants: Vec<wisp_server::desktop::ProjectGrants>,
    /// Tools that are allowed in every project.
    global_grants: Vec<String>,
    /// The open project's approval mode and tool rules, with the project
    /// they are for.
    approvals: Option<(String, wisp_server::desktop::ProjectApprovals)>,
    /// What went wrong on a Settings page other than Models.
    settings_error: Option<String>,
    /// Every skill there is, as the Skills page lists them; `None` until read.
    skills: Option<Vec<wisp_server::desktop::SkillRow>>,
    skills_search: Entity<InputState>,
    /// The skill whose tags are being changed, and the field they are typed in.
    skill_tags_editing: Option<String>,
    skill_tags_input: Entity<InputState>,
    /// The field that narrows the list of Settings pages.
    settings_search: Entity<InputState>,
    /// The bundled connectors, as the Connections page lists them.
    connectors: Option<Vec<wisp_server::desktop::ConnectorRow>>,
    connectors_search: Entity<InputState>,
    /// The connector whose tools and links are unfolded.
    open_connector: Option<String>,
    /// The SSH hosts and the default place for new work; `None` until read.
    environments: Option<wisp_server::desktop::EnvironmentsView>,
    /// Python, R and Node under the data folder: their state, the tools
    /// chosen to install, and whether the page is following an install.
    toolchain: Option<wisp_dto::ToolchainStatus>,
    toolchain_choice: Vec<&'static str>,
    toolchain_following: bool,
    /// The first-run guide: its step, the key typed on the model step,
    /// whether the key is being saved, and why it was refused.
    onboarding_step: usize,
    onboarding_key: Entity<InputState>,
    onboarding_saving: bool,
    onboarding_error: Option<String>,
    /// What the Network page last read from the host, its fields, and
    /// whether the last Save was kept.
    network: Option<wisp_dto::NetworkSettings>,
    network_fields: [Entity<InputState>; 7],
    network_saved: bool,
    /// The host the form changes or the remove question is about; `None`
    /// in the form means a new one.
    env_target: Option<String>,
    /// The hosts being checked right now.
    env_checking: std::collections::HashSet<String>,
    /// The project whose settings dialog is open; its fields (name,
    /// description, rules); whether they hold what is saved yet; and why
    /// the last Save was refused.
    project_form: Option<String>,
    /// Name, description, rules, and the days succeeded runs, failed runs
    /// and orphaned server files are kept.
    project_fields: [Entity<InputState>; 6],
    project_form_ready: bool,
    project_form_error: Option<String>,
    /// The host form's fields: name, address, user, port, key file, jump
    /// host, notes.
    env_fields: [Entity<InputState>; 7],
    /// The host in the form signs in with a password, which is typed here
    /// (masked) and kept in the keyring; whether one is saved already.
    env_password_sign_in: bool,
    env_password: Entity<InputState>,
    env_has_password: bool,
    /// The user's MCP servers; `None` until read.
    /// Requests kept under a name, for the composer's menu.
    quick_actions: Vec<wisp_server::desktop::QuickAction>,
    /// The action in the form or the remove question; empty is a new one.
    quick_editing: Option<String>,
    /// Name, request.
    quick_fields: [Entity<InputState>; 2],
    /// The workflow the action being written starts; empty for none.
    quick_workflow: String,
    /// The workspace folder for the next start, on the General page.
    workspace_choice: Entity<InputState>,
    /// Whether new projects keep their own database; `None` until read.
    separate_storage: Option<bool>,
    /// The projects hidden while recording, and whether that is on.
    privacy: wisp_dto::project_browser::PrivacyMode,
    /// The projects ticked in the privacy dialog.
    privacy_picks: HashSet<String>,
    /// The workspace folder typed was kept for the next start.
    workspace_saved: bool,
    /// Your command hooks, as saved.
    command_hooks: Vec<wisp_dto::CommandHook>,
    /// The open project's `.wisp/hooks.json`, when it has one.
    project_hook_file: Option<wisp_dto::ProjectHooks>,
    /// The hook being written: its place in the list (none for a new one)
    /// and when it runs.
    hook_editing: Option<(Option<usize>, wisp_dto::HookEvent)>,
    /// Tools, command and timeout of the hook being written.
    hook_fields: [Entity<InputState>; 3],
    /// The decision model as the host keeps it; `None` until it answered.
    decision: Option<wisp_dto::DecisionModelSettings>,
    /// The form's switches: on or off, the service, how it thinks.
    decision_draft: wisp_dto::DecisionModelSettings,
    /// Base URL, model, timeout, API key.
    decision_fields: [Entity<InputState>; 4],
    /// What the last save or test said.
    decision_note: Option<String>,
    decision_testing: bool,
    /// The browser tools as the host has them; `None` until it answered.
    browser: Option<wisp_server::desktop::BrowserSettings>,
    /// Blocked host and why, preferred host and why.
    browser_fields: [Entity<InputState>; 4],
    /// What is asked or told about the browser over the shown project.
    browser_prompts: crate::browser::Prompts,
    /// The project those are about.
    browser_project: Option<String>,
    /// Asks the host about the browser every two seconds.
    browser_watch: Option<Task<()>>,
    /// The workflow templates.
    workflows: Vec<wisp_server::desktop::Workflow>,
    /// The workflow in the form or the remove question; empty is a new one.
    workflow_editing: Option<String>,
    /// Name, goal.
    workflow_fields: [Entity<InputState>; 3],
    /// What a workflow written from skills is for.
    workflow_request: Entity<InputState>,
    /// The skills that can be turned into a workflow, and those picked.
    workflow_skills: Vec<wisp_server::desktop::SkillRow>,
    workflow_skill_picks: Vec<String>,
    /// A workflow is being written from skills.
    workflow_converting: bool,
    /// The terminals drawn at the foot of the conversation.
    terminal: terminal_view::TerminalDock,
    /// The tasks of the workflow in the form.
    workflow_tasks: Vec<settings_workflows::TaskDraft>,
    /// How the workflow in the form is approved when it runs as agents.
    workflow_approval: wisp_server::desktop::WorkflowApproval,
    /// The task picked on the workflow's map, to join to the next one
    /// clicked.
    workflow_pick: Option<usize>,
    /// While a task is dragged on the workflow's map: that task, and where
    /// the pointer is on the map, for the line drawn to it.
    workflow_drag: Option<(usize, Point<Pixels>)>,
    /// Remote web access as the host has it; `None` until it answered.
    remote_access: Option<wisp_server::desktop::RemoteAccess>,
    /// The web access link is shown as a QR code.
    remote_qr_shown: bool,
    /// Relay address, relay token.
    remote_fields: [Entity<InputState>; 2],
    /// Names the latest round of looks at the tunnel.
    remote_watch: u64,
    /// The StickS3 desk display on the same page: its listener, the address
    /// and port typed, and the token once shown.
    device: Option<wisp_server::desktop::DeviceBridge>,
    device_fields: [Entity<InputState>; 2],
    device_token: Option<String>,
    device_note: Option<Result<String, String>>,
    /// The installed plugins; `None` until the host answered.
    plugins: Option<Vec<wisp_server::desktop::PluginRow>>,
    plugin_installing: bool,
    /// The community directory of skills, once read.
    community_skills: Option<wisp_dto::CommunitySkillCatalog>,
    /// The skill packages found at the address asked about.
    skill_candidates: Vec<wisp_dto::SkillStoreCandidate>,
    /// A skill directory, preview or install is under way.
    skill_store_busy: bool,
    /// What the last of those said.
    skill_store_note: Option<String>,
    /// A GitHub address and the exact ref, for skills.
    github_skill_fields: [Entity<InputState>; 2],
    /// The address and SHA-256 of a plugin to download.
    plugin_address: [Entity<InputState>; 2],
    /// The plugin the remove question is about.
    plugin_removing: Option<String>,
    /// The pet as the host keeps it; `None` until it answered.
    pet: Option<wisp_server::desktop::Pet>,
    pet_folder: Entity<InputState>,
    /// The animated pet, a view of its own.
    pet_view: Entity<pet_view::PetView>,
    /// The personas a conversation can be started as.
    experts: Vec<wisp_server::desktop::Expert>,
    /// The expert in the form or the remove question; empty is a new one.
    expert_editing: Option<String>,
    /// Name, description, instructions.
    expert_fields: [Entity<InputState>; 3],
    /// The model profile the expert in the form runs on; empty follows the
    /// conversation's.
    expert_model: String,
    /// The skills and connectors the expert in the form may use; `None` is
    /// every one.
    expert_skills: Option<Vec<String>>,
    expert_connectors: Option<Vec<String>>,
    /// The ACP agents, the one in the form or the remove question (empty
    /// is a new one), its name, program and arguments, the one being
    /// tested, and what each test found.
    acp_agents: Option<Vec<wisp_server::desktop::AcpAgentRow>>,
    acp_editing: Option<String>,
    acp_fields: [Entity<InputState>; 3],
    acp_testing: Option<String>,
    /// How signing in to each agent went.
    acp_signed: std::collections::HashMap<String, Result<String, String>>,
    /// The ACP mode or option whose list is open in the composer.
    acp_setting_open: Option<String>,
    /// The subscriptions and who is signed in to each; the sign-in under
    /// way, where it stands, the address pasted for it, and how the last
    /// one ended.
    subscriptions: Vec<wisp_server::desktop::SubscriptionRow>,
    sign_in: Option<(
        wisp_server::desktop::Subscription,
        wisp_server::desktop::SignInChallenge,
    )>,
    sign_in_state: Option<wisp_server::desktop::SignInState>,
    sign_in_redirect: Entity<InputState>,
    sign_in_note: Option<Result<String, String>>,
    /// The ChatGPT (Codex) accounts kept, each one's usage once asked for,
    /// whether it is being asked for, and the account a remove question
    /// is about.
    codex_accounts: Vec<wisp_server::desktop::CodexAccount>,
    codex_usage:
        std::collections::HashMap<String, Result<wisp_server::desktop::CodexUsage, String>>,
    codex_usage_checking: bool,
    codex_removing: Option<String>,
    /// WeChat on the Chat apps page: where it stands, the QR code being
    /// scanned and how far the scan got, which binding attempt is current,
    /// and how the last change went.
    /// The WeChat bot that reaches the projects, and the research
    /// assistant's own.
    weixin: crate::channels::WeixinCard,
    assistant_weixin: crate::channels::WeixinCard,
    /// Feishu on the same page: where it stands, its form (App ID, a new
    /// secret, the owner) and service, the scan that makes its app, and how
    /// the last change went.
    feishu: Option<wisp_server::desktop::FeishuStatus>,
    feishu_fields: [Entity<InputState>; 3],
    feishu_lark: bool,
    feishu_qr: Option<wisp_server::desktop::FeishuBind>,
    feishu_bind_id: u64,
    feishu_note: Option<Result<String, String>>,
    /// Wisp Lab on the same page: where this computer stands, the join form
    /// (relay, token, name, lab name, invite, and a replacement token), the
    /// latest invite code made, the member a remove question is about, and
    /// how the last change went.
    lab: Option<wisp_server::desktop::LabStatus>,
    lab_fields: [Entity<InputState>; 6],
    lab_invite: Option<String>,
    lab_removing: Option<String>,
    lab_busy: bool,
    lab_note: Option<Result<String, String>>,
    /// Sync of the project in the settings dialog: how it stands (`Some`
    /// once read; inside, `None` when it is not synchronized), whether a
    /// sync runs, how the last went, whether it ended in a conflict, and
    /// which side the conflict question would keep.
    project_sync: Option<Option<wisp_server::desktop::ProjectSyncInfo>>,
    project_sync_busy: bool,
    project_sync_note: Option<Result<String, String>>,
    project_sync_conflict: bool,
    project_sync_replace: Option<bool>,
    /// Where projects are synchronized, on the Remote access page: the
    /// relay, a new token and the folder typed, and the backend chosen.
    sync_settings: Option<wisp_server::desktop::SyncSettings>,
    sync_fields: [Entity<InputState>; 3],
    sync_backend: String,
    sync_note: Option<Result<String, String>>,
    /// The dialog that joins a project by its device code.
    join_code: Entity<InputState>,
    join_busy: bool,
    join_note: Option<String>,
    /// The New project dialog's name and description, whether the folder
    /// gets the standard research layout, and why the last try failed.
    new_project_name: Entity<InputState>,
    new_project_description: Entity<InputState>,
    new_project_layout: bool,
    new_project_note: Option<String>,
    /// The open project's exploration round, the dialog that names a new
    /// exploration and the answer it starts from, what keeping one would
    /// bring in, the question asked before one is given up, and how the
    /// last action went.
    explorations: Vec<wisp_dto::ExplorationSummary>,
    /// The task batches the open conversation's agent delegated.
    batches: Vec<wisp_server::desktop::DelegatedBatch>,
    /// The conversation `batches` were read for; until it is the open one,
    /// the calls that delegated them stay ordinary tool rows.
    batches_for: Option<String>,
    /// The card each transcript row that hands work to other agents is
    /// drawn as, by row, worked out again whenever the rows, the batches or
    /// the conversation list change.
    agent_cards: HashMap<usize, crate::agents::Card>,
    /// The rows that draw a run's card, with the run and the shape of what
    /// the card shows, worked out again whenever the rows or the
    /// conversation's runs change.
    run_card_rows: HashMap<usize, (String, u64)>,
    /// The cards of runs that no row names, by the row they are drawn with:
    /// where, the run and the shape of what the card shows.
    own_run_cards: HashMap<usize, Vec<(crate::run_cards::Anchor, String, u64)>>,
    /// Ended runs whose card of its own was dismissed while the window is
    /// open, as the WebView keeps them.
    dismissed_runs: HashSet<String>,
    /// Runs the user moved to the background here, until a snapshot says so.
    backgrounded_runs: HashSet<String>,
    /// The latest work over many items the open conversation ran.
    swarm: Option<wisp_dto::SwarmProgress>,
    batches_request: u64,
    /// The task whose token budget is being changed, by batch and task id,
    /// and the field it is typed in.
    budget_target: Option<(String, String)>,
    budget_input: Entity<InputState>,
    /// Token budgets given to tasks for their batch's next run.
    budget_changes: HashMap<(String, String), u32>,
    explore_name: Entity<InputState>,
    explore_from: Option<TurnRef>,
    explore_preview: Option<(String, wisp_server::desktop::PromotionPreview)>,
    explore_question: Option<explore_view::ExploreQuestion>,
    explore_busy: bool,
    explore_note: Option<Result<String, String>>,
    acp_reports:
        std::collections::HashMap<String, Result<wisp_server::desktop::AcpAgentReport, String>>,
    mcp_servers: Option<Vec<wisp_server::desktop::McpServerRow>>,
    /// The server the form changes or the remove question is about; `None`
    /// in the form means a new one.
    mcp_editing: Option<String>,
    /// The form is for a server over HTTP rather than a local program.
    mcp_http: bool,
    mcp_name: Entity<InputState>,
    mcp_target: Entity<InputState>,
    mcp_token: Entity<InputState>,
    /// The server's headers or environment variables, a line each.
    mcp_pairs: Entity<InputState>,
    /// An HTTP server's own route.
    mcp_route: Entity<InputState>,
    /// What testing a server found: `None` while it runs, then its tools
    /// or why it failed.
    mcp_tested: HashMap<String, Option<Result<Vec<String>, String>>>,
    /// The service keys, as the Credentials page lists them, each with the
    /// field a new value is typed into.
    credentials: Vec<(wisp_server::desktop::CredentialRow, Entity<InputState>)>,
    /// Credentials of your own, and the fields for adding one: name,
    /// environment variable and value.
    custom_credentials: Vec<wisp_server::desktop::CustomCredential>,
    custom_credential_fields: [Entity<InputState>; 3],
    /// Tokens used, as the Usage page shows them; `None` until read.
    usage: Option<wisp_server::desktop::UsageView>,
    /// What Wisp keeps and where; `None` until read.
    storage: Option<wisp_server::desktop::StorageView>,
    /// What the open conversation saved to the favorites, newest first.
    saved: Vec<LibraryItem>,
    saved_request: u64,
    /// The Favorites page's list; `None` until it was read.
    favorites: Option<Vec<LibraryItemSummary>>,
    favorites_request: u64,
    favorites_filter: crate::favorites::Filter,
    favorites_search: Entity<InputState>,
    /// The item open on the Favorites page, and its picture if it has one.
    favorite: Option<Favorite>,
    favorite_image: Option<std::sync::Arc<Image>>,
    /// The open item's code versions, the original first, the one shown,
    /// the editor for a new one and the code that edit started from.
    favorite_versions: Vec<wisp_server::desktop::LibraryItemVersion>,
    favorite_shown: usize,
    favorite_code: Entity<InputState>,
    favorite_edit_base: String,
    /// Pictures the open project's answers point at, by path. `None` is a
    /// picture that is being read or could not be.
    inline_images: HashMap<String, Option<std::sync::Arc<Image>>>,
    /// The files each turn of the open conversation recorded, as the host
    /// lists them for the answers that close those turns.
    reply_artifacts: Vec<wisp_server::desktop::ReplyArtifact>,
    /// Those files by the row of the answer they go under, worked out again
    /// whenever the rows or the files change.
    generated: HashMap<usize, crate::generated::Reply>,
    /// The open "Generated" lists and folders while the conversation stays
    /// loaded: an answer's key with "" for the list itself, or a folder's
    /// path in it.
    generated_open: HashSet<(String, String)>,
    /// Only the latest request may fill `reply_artifacts`.
    reply_artifacts_request: u64,
    /// The picture being previewed in the Files panel, with its path.
    file_image: Option<(String, std::sync::Arc<Image>)>,
    /// The document being previewed, turned into text, with its path.
    file_document: Option<(String, Result<String, String>)>,
    /// The PDF being previewed, its first pages drawn, with its path and
    /// the line that says how many there are.
    /// Each page with its height over its width.
    file_pages: Option<(
        String,
        Result<(Vec<(std::sync::Arc<Image>, f32)>, String), String>,
    )>,
    /// The alignment or FASTA file being previewed, read once, and the
    /// page of columns shown.
    file_sequences: Option<(String, std::sync::Arc<crate::sequences::Alignment>)>,
    sequence_page: usize,
    /// The molecule or structure being previewed, read once; how far it is
    /// turned (yaw, pitch); and where a drag that turns it last was.
    file_structure: Option<(String, std::sync::Arc<crate::molecules::Structure>)>,
    structure_turn: (f32, f32),
    structure_drag: Option<Point<Pixels>>,
    /// How the previewed picture is zoomed and scrolled; back to the
    /// preview's width for each picture opened, as in the WebView.
    picture: crate::picture::View,
    /// The picture preview's place in the window and the picture's own
    /// size in pixels, as last drawn, for the pointer's events.
    picture_geometry: std::rc::Rc<std::cell::Cell<Option<(Bounds<Pixels>, (f32, f32))>>>,
    /// Where a drag that moves the picture last was.
    picture_pan: Option<Point<Pixels>>,
    /// The region being marked on the picture, or marked and waiting for
    /// its note.
    picture_mark: Option<crate::picture::Mark>,
    /// Notes on regions of pictures, while the window is open.
    picture_pins: crate::picture::Pins,
    /// The note being written for the marked region.
    pin_note: Entity<InputState>,
    /// A marked region is being cut out of the picture for the message.
    picture_cutting: bool,
    favorite_request: u64,
    rename: Entity<InputState>,
    settings: settings_view::SettingsForm,
    composer: Entity<InputState>,
    search: Entity<InputState>,
    rows: Vec<Item>,
    history: Vec<Item>,
    before: Option<i64>,
    /// How many of the user's messages come before the rows held: the
    /// host's `user_offset` for the earliest page loaded. Entry numbers
    /// start after it, so they do not change as earlier pages load.
    earlier_users: usize,
    /// Where the latest page started (its `next_before_seq` and how many
    /// of the user's messages came before it) when `history` was loaded
    /// behind it; see [`crate::outline::earlier_pages_hold`].
    history_under: (Option<i64>, usize),
    /// Counts the times `history` was dropped, so an earlier page asked
    /// for before that is not put in front of rows it no longer joins.
    history_epoch: u64,
    list: ListState,
    /// The example conversations that ship with Wisp, once listed.
    demos: Vec<wisp_server::desktop::DemoInfo>,
    /// The example being read.
    demo: Option<wisp_server::desktop::Demo>,
    /// Its rows, drawn as they scroll into view.
    demo_list: ListState,
    /// Its folded rows that are open.
    demo_open: HashSet<usize>,
    /// Why the examples could not be read or copied.
    demo_note: Option<String>,
    /// Open dismissible surfaces; the only place that decides what Escape closes.
    layers: EscapeStack,
    files: FilesState,
    file_view: Entity<InputState>,
    /// The text of a host's file being looked at, read only.
    remote_text: Entity<InputState>,
    /// The picture of a host's file being looked at, by its key.
    remote_image: Option<(String, std::sync::Arc<Image>)>,
    /// The script being edited in the preview, when the previewed file is
    /// one that can be.
    file_edit: Option<crate::files::Editing>,
    /// Unsaved edits of every file, kept while the window is open.
    file_drafts: crate::files::Drafts,
    /// The window may close over unsaved edits: the user said so.
    close_confirmed: bool,
    expanded_run: Option<String>,
    /// Which tab the side panel shows.
    panel_tab: PanelTab,
    /// The side panel's tabs that have not been taken away; saved.
    kept_tabs: Vec<PanelTab>,
    /// The run whose workspace is being asked about, and whether cleaning
    /// it now would lose outputs never brought back.
    run_cleaning: Option<(String, bool)>,
    /// What a finished run left on its SSH host, being looked through.
    run_review: Option<crate::run_review::RunReview>,
    run_review_filter: Entity<InputState>,
    /// The transfers above the composer show as one line.
    transfers_folded: bool,
    /// The SSH hosts the open conversation's work may run on, by alias;
    /// `None` until read.
    conversation_hosts: Option<Vec<String>>,
    /// A host is being attached or taken away.
    hosts_busy: bool,
    /// The host whose storage form is open, and whether what it shows was
    /// saved rather than the defaults.
    storage_host: Option<(String, bool)>,
    /// Data folder, run folders, results folder.
    storage_fields: [Entity<InputState>; 3],
    /// The Python and Rscript paths kept on a host, in its interpreter form.
    interpreter_fields: [Entity<InputState>; 2],
    /// One host's runs, or the files the project placed there, under its
    /// card in the Environments tab.
    host_detail: Option<hosts_view::HostDetail>,
    /// The placed file whose deletion is being asked about.
    staged_removing: Option<wisp_server::desktop::StagedFile>,
    artifacts: Vec<ArtifactRow>,
    /// The Artifacts tab lists the whole project instead of the conversation.
    artifacts_all: bool,
    /// The Artifacts tab shows its files under their folders.
    artifacts_by_folder: bool,
    /// Which tool call wrote an artifact, once asked: its id and what the
    /// host found.
    artifact_origin: Option<(
        String,
        Result<Option<wisp_server::desktop::FileOrigin>, String>,
    )>,
    /// Only the latest artifact request may fill the list.
    artifacts_request: u64,
    ready: bool,
    pending: bool,
    loading_history: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
    poll: Option<Task<()>>,
    watching: bool,
    watch: Option<Task<()>>,
}

/// Tells the user that a Run moved to the background ended, through the
/// notification and sound chosen in Settings, unless its conversation is on
/// screen in the focused window: one notification for the whole app,
/// however many windows are open.
fn tell_run_ended(ended: &wisp_server::desktop::BackgroundRunEnded, cx: &mut App) {
    let mut on_screen = false;
    let mut teller = None;
    for handle in cx.windows() {
        let seen = handle.update(cx, |root, window, cx| {
            let root = root.downcast::<gpui_component::Root>().ok()?;
            let workspace = root.read(cx).view().clone().downcast::<Workspace>().ok()?;
            Some((window.is_window_active(), workspace))
        });
        let Ok(Some((active, workspace))) = seen else {
            continue;
        };
        on_screen |= active && workspace.read(cx).shows(&ended.session);
        teller.get_or_insert(workspace);
    }
    let Some(teller) = teller else {
        return;
    };
    let teller = teller.read(cx);
    if !crate::notify::run_end_wanted(teller.notify, on_screen) {
        return;
    }
    crate::notify::post_run_end(
        teller.notify,
        teller.notify_tone,
        &ended.status,
        &present::one_line(&ended.conversation),
        &ended.title,
    );
}

fn tone_color(tone: Tone, cx: &App) -> Hsla {
    match tone {
        Tone::Neutral => cx.theme().muted_foreground,
        Tone::Active => cx.theme().info,
        Tone::Success => cx.theme().success,
        Tone::Danger => cx.theme().danger,
        Tone::Attention => cx.theme().warning,
    }
}

fn chip(label: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    div()
        .flex_shrink_0()
        .text_xs()
        .text_color(tone_color(tone, cx))
        .child(label.into())
}

impl Workspace {
    pub fn new(backend: Backend, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(3, 9)
                .placeholder(crate::settings::SendKey::default().placeholder())
        });
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Search conversations…")));
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Conversation title")));
        let pin_note =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Write a short comment…")));
        let folder_name = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Group name")));
        let delete_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Type the project's name")));
        let memo_title =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Title (optional)")));
        let memo_body = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder(tr("A prompt you reuse, a server address, a reminder…"))
        });
        let memo_search = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Search memos")));
        let settings = settings_view::SettingsForm::new(window, cx);
        let palette_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("Search conversations, projects and files…"))
        });
        // In-editor search would be a nested surface outside the Escape stack.
        let file_view = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("text")
                .searchable(false)
        });
        let remote_text = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("text")
                .searchable(false)
        });
        let memory_text = cx.new(|cx| InputState::new(window, cx).multi_line(true));
        let favorite_code = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("text")
                .searchable(false)
        });
        let steps_input = cx.new(|cx| InputState::new(window, cx));
        let mcp_name = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Lab notebook")));
        let env_fields = ["gpu", "10.0.0.8", "", "22", "~/.ssh/id_ed25519", "", ""]
            .map(|hint| cx.new(|cx| InputState::new(window, cx).placeholder(hint)));
        let env_password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let network_fields = [
            "http://127.0.0.1:7890",
            "http://127.0.0.1:7890",
            "http://127.0.0.1:7890",
            "http://127.0.0.1:7890",
            "https://conda.example.org/conda-forge",
            "https://pypi.example.org/simple",
            "/etc/ssl/certs/lab-ca.pem",
        ]
        .map(|hint| cx.new(|cx| InputState::new(window, cx).placeholder(hint)));
        let project_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(true)
                    .placeholder(tr("Always report effect sizes with confidence intervals."))
            }),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Off"))),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Off"))),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Off"))),
        ];
        let find_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Find in this conversation…")));
        let file_name_input = cx.new(|cx| InputState::new(window, cx));
        let custom_credential_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Name, for you"))),
            cx.new(|cx| InputState::new(window, cx).placeholder("LAB_API_TOKEN")),
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(tr("Value"))
            }),
        ];
        let file_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("Search by name across the project…"))
        });
        let aside_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(tr("Ask about this conversation…")));
        let plan_feedback = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("Tell Wisp what to change in this plan."))
        });
        let journey_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Filter this month…")));
        let recap_time = cx.new(|cx| InputState::new(window, cx).placeholder("09:00"));
        let console_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder(tr("Code to run in this interpreter…"))
        });
        let budget_input = cx.new(|cx| InputState::new(window, cx).placeholder(tr("Tokens")));
        let mandate_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Literature watch"))),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Papers filed, 5, per week"))),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Your answer…"))),
        ];
        let recap_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
        ];
        let journal_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("The reads are clean"))),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
        ];
        let mcp_target = cx.new(|cx| InputState::new(window, cx));
        let schedule_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
        ];
        let paper_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
        ];
        let paper_item_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
        ];
        let paper_source_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
        ];
        let paper_anchor_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
        ];
        let paper_passage_field = cx.new(|cx| InputState::new(window, cx).multi_line(true));
        let paper_waiver_field = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(tr("Why it can stand, for whoever reads the record"))
        });
        let browser_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("example.com")),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Reason (optional)"))),
            cx.new(|cx| InputState::new(window, cx).placeholder("example.org")),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Reason (optional)"))),
        ];
        let workflow_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
        ];
        let workflow_request = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder(tr("What the workflow is for"))
        });
        let sync_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("https://relay.example")),
            cx.new(|cx| InputState::new(window, cx).masked(true)),
            cx.new(|cx| InputState::new(window, cx).placeholder("/Volumes/Shared/Wisp")),
        ];
        let join_code = cx.new(|cx| InputState::new(window, cx).placeholder("wisp-sync:…"));
        let new_project_name =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Project name")));
        let new_project_description = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder(tr("What the project is about (optional)"))
        });
        let explore_name = cx.new(|cx| InputState::new(window, cx));
        let lab_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("https://relay.example")),
            cx.new(|cx| InputState::new(window, cx).masked(true)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).masked(true)),
        ];
        let feishu_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("cli_…")),
            cx.new(|cx| InputState::new(window, cx).masked(true)),
            cx.new(|cx| InputState::new(window, cx).placeholder("ou_…")),
        ];
        let device_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("192.168.1.20")),
            cx.new(|cx| InputState::new(window, cx).placeholder("18766")),
        ];
        let remote_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("https://relay.example")),
            cx.new(|cx| InputState::new(window, cx).masked(true)),
        ];
        let decision_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).masked(true)),
        ];
        let pet_folder =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("A folder holding pet.json")));
        let pet_view = {
            let workspace = cx.entity().downgrade();
            cx.new(|_| pet_view::PetView::new(workspace))
        };
        let quick_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
        ];
        let hook_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("write|edit")),
            cx.new(|cx| InputState::new(window, cx).placeholder("./.wisp/hooks/check.sh")),
            cx.new(|cx| InputState::new(window, cx).placeholder("60")),
        ];
        let archive_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
            cx.new(|cx| InputState::new(window, cx)),
            // In-editor search would be a nested surface outside the Escape stack.
            cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor("text")
                    .searchable(false)
            }),
        ];
        let merge_summary = cx.new(|cx| InputState::new(window, cx).multi_line(true));
        let handoff_fields = [
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr(
                    "For example: keep the unresolved QC thresholds and exact file paths.",
                ))
            }),
            cx.new(|cx| {
                InputState::new(window, cx).multi_line(true).placeholder(tr(
                    "The generated note appears here. Edit it freely before starting.",
                ))
            }),
        ];
        let reuse_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
            cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr("Find a conversation in this project"))
            }),
        ];
        let expert_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).multi_line(true)),
        ];
        let acp_fields = [
            cx.new(|cx| InputState::new(window, cx)),
            cx.new(|cx| InputState::new(window, cx).placeholder("npx")),
            cx.new(|cx| InputState::new(window, cx)),
        ];
        let mcp_token = cx.new(|cx| InputState::new(window, cx).masked(true));
        let mcp_pairs = cx.new(|cx| InputState::new(window, cx).multi_line(true));
        let github_skill_fields = [
            cx.new(|cx| {
                InputState::new(window, cx).placeholder("https://github.com/owner/repository")
            }),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Branch, tag or commit"))),
        ];
        let workspace_choice = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                crate::settings::default_workspace()
                    .map(|folder| folder.display().to_string())
                    .unwrap_or_default(),
            )
        });
        let storage_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("~/wisp/project/data")),
            cx.new(|cx| InputState::new(window, cx).placeholder(".wisp-science/runs")),
            cx.new(|cx| InputState::new(window, cx).placeholder("remote/host")),
        ];
        let interpreter_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder("/opt/conda/bin/python")),
            cx.new(|cx| InputState::new(window, cx).placeholder("/usr/bin/Rscript")),
        ];
        let font_fields = [
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Wisp's own"))),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Wisp's own"))),
        ];
        let run_review_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Names containing…")));
        let plugin_address = [
            cx.new(|cx| InputState::new(window, cx).placeholder("https://example.org/plugin.zip")),
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Its SHA-256, as published"))),
        ];
        let mcp_route =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Follows the Network page")));
        let sign_in_redirect = cx.new(|cx| {
            InputState::new(window, cx).placeholder("http://localhost:1455/auth/callback?code=…")
        });
        let onboarding_key = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(tr("Paste the DeepSeek API key"))
        });
        let skills_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Find a skill…")));
        let skill_tags_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Tags, separated by commas")));
        let settings_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Search settings…")));
        let connectors_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Find a connector or a tool…")));
        let favorites_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Search your favorites…")));
        let handle = window.window_handle();
        let weak = cx.weak_entity();
        let subscriptions = vec![
            cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let now = this.composer.read(cx).value().to_string();
                    // `@` or `#` where a word starts asks to mention
                    // something: the picker opens and the character goes.
                    if crate::compose::mention_typed(&this.composer_before, &now) {
                        let kept = this.composer_before.clone();
                        this.fill_composer(kept, window, cx);
                        this.toggle_attach_palette(window, cx);
                        return;
                    }
                    this.composer_before = now.clone();
                    this.follow_slash(&now, window, cx);
                    this.state.save_draft(now);
                    cx.notify();
                }
                // Let the input component and platform IME consume composition.
                // Platform secondary+Enter always sends; plain Enter sends
                // through `SendMessage` when that is the chosen key.
                if matches!(event, InputEvent::PressEnter { secondary: true }) {
                    this.send(window, cx);
                }
            }),
            // Search runs in the host: titles and message text, archive included.
            // Return keeps the budget typed for a delegated task.
            // Return lists the names that contain what was typed.
            cx.subscribe_in(&run_review_filter, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.list_run_review(false, window, cx);
                }
            }),
            cx.subscribe_in(&budget_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_budget(window, cx);
                }
            }),
            cx.subscribe_in(&search, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.load_sessions(window, cx);
                }
            }),
            cx.subscribe_in(
                &palette_input,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::Change => this.search_palette(window, cx),
                    InputEvent::PressEnter { .. } => this.activate_palette(None, window, cx),
                    _ => {}
                },
            ),
            // Filtering is done as the page is drawn.
            cx.subscribe_in(&journey_search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            // The archive's draft follows its fields as they are typed in.
            cx.subscribe_in(&archive_fields[0], window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.note_archive_edit(cx);
                }
            }),
            cx.subscribe_in(&archive_fields[1], window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.note_archive_edit(cx);
                }
            }),
            cx.subscribe_in(&archive_fields[2], window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.note_archive_edit(cx);
                }
            }),
            cx.subscribe_in(&archive_fields[3], window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.note_archive_edit(cx);
                }
            }),
            // Whether an edit of saved code can be saved shows as it is typed.
            cx.subscribe_in(&favorite_code, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&aside_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.ask_aside(window, cx);
                }
            }),
            // The merge button follows whether a summary is written.
            cx.subscribe_in(&merge_summary, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            // And Start follows whether there is a handoff note.
            cx.subscribe_in(&handoff_fields[1], window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            // An edited experience draft is not the one kept, and the
            // buttons follow whether there is one.
            cx.subscribe_in(&reuse_fields[1], window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    if let Some(dialog) = this.reuse.as_mut() {
                        dialog.edited();
                    }
                    cx.notify();
                }
            }),
            // The destinations follow the search.
            cx.subscribe_in(&reuse_fields[2], window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            // The feedback button follows what is written; Return sends it.
            cx.subscribe_in(
                &plan_feedback,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => {
                        let asking = this.state.snapshot.as_ref().and_then(|snapshot| {
                            snapshot
                                .approvals
                                .iter()
                                .find(|approval| {
                                    controls::plan_approval(&approval.message).is_some()
                                })
                                .map(|approval| approval.approval_id.clone())
                        });
                        if let Some(approval) = asking {
                            this.send_plan_feedback(approval, window, cx);
                        }
                    }
                    _ => {}
                },
            ),
            // Each change to an edited script is kept as its draft.
            cx.subscribe_in(&file_view, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.note_file_edit(cx);
                }
            }),
            cx.subscribe_in(&file_name_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_file_name(window, cx);
                }
            }),
            cx.subscribe_in(&file_search, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search_files_by_name(window, cx);
                }
            }),
            cx.subscribe_in(&find_input, window, |this, _, event, _, cx| match event {
                InputEvent::Change => this.search_find(cx),
                // Return goes back through the conversation; with the
                // modifier it goes forward.
                InputEvent::PressEnter { secondary } => this.step_find(!secondary, cx),
                _ => {}
            }),
            cx.subscribe_in(&rename, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_rename(window, cx);
                }
            }),
            // Return keeps the note on the marked region.
            cx.subscribe_in(&pin_note, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add_picture_pin(window, cx);
                }
            }),
            // Filtering is done here, over the list already read.
            cx.subscribe_in(
                &settings_search,
                window,
                |this, _, event, window, cx| match event {
                    // The list is filtered as it is drawn.
                    InputEvent::Change => cx.notify(),
                    // Return opens the first page that is left.
                    InputEvent::PressEnter { .. } => {
                        let query = this.settings_search.read(cx).value().to_string();
                        if let Some(first) = crate::settings::search_sections(&query).first() {
                            this.show_settings_section(*first, window, cx);
                        }
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&memo_search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&skills_search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&connectors_search, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&steps_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_steps(window, cx);
                }
            }),
            cx.subscribe_in(&favorites_search, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.load_favorites(window, cx);
                }
            }),
            cx.subscribe_in(&delete_name_input, window, |this, _, event, window, cx| {
                match event {
                    // The button follows what is typed.
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => this.delete_project_folder(window, cx),
                    _ => {}
                }
            }),
            cx.subscribe_in(&folder_name, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.save_folder_name(window, cx);
                }
            }),
            // Window-level and ahead of key dispatch: Escape closes the topmost
            // layer wherever focus is, including nowhere. Platform IME handling
            // runs earlier still, so cancelling a composition is not seen here.
            cx.intercept_keystrokes(move |event, window, cx| {
                if window.window_handle() != handle {
                    return;
                }
                let key = &event.keystroke.key;
                let held = event.keystroke.modifiers;
                // A terminal with focus keeps Escape, and Ctrl chords off
                // macOS, for its shell: an inner surface consuming them
                // before the window does.
                let in_terminal = weak
                    .read_with(cx, |this, _| this.terminal.focus.is_focused(window))
                    .unwrap_or(false);
                if in_terminal
                    && crate::layers::terminal_keeps(key, held.control, cfg!(target_os = "macos"))
                {
                    return;
                }
                // A picture pasted into the composer is attached; text
                // goes on to the composer as ever.
                let others_held = held.alt
                    || held.function
                    || if cfg!(target_os = "macos") {
                        held.control
                    } else {
                        held.platform
                    };
                if compose::is_paste(key, held.secondary(), held.shift, others_held) {
                    if weak
                        .update(cx, |this, cx| this.paste_picture(window, cx))
                        .unwrap_or(false)
                    {
                        cx.stop_propagation();
                    }
                    return;
                }
                let handled = if is_escape(key, held.modified()) {
                    weak.update(cx, |this, cx| this.dismiss_top(window, cx))
                } else if let Some(delta) = palette::step(key, held.modified()) {
                    // Arrow keys belong to the palette only while it is open.
                    weak.update(cx, |this, cx| this.step_palette(delta, cx))
                } else {
                    // Whatever is held besides the platform's command key
                    // and Shift.
                    let others = held.alt
                        || held.function
                        || if cfg!(target_os = "macos") {
                            held.control
                        } else {
                            held.platform
                        };
                    match shortcut(key, held.secondary(), held.shift, others) {
                        Some(command) => {
                            weak.update(cx, |this, cx| this.run_shortcut(command, window, cx))
                        }
                        None => return,
                    }
                };
                if handled.unwrap_or(false) {
                    cx.stop_propagation();
                }
            }),
            // A system change only matters while the system is followed.
            cx.observe_window_appearance(window, |this, window, cx| {
                crate::theme::apply(this.appearance, window, cx)
            }),
            cx.observe_window_activation(window, |this, window, cx| this.mark_seen(window, cx)),
        ];
        let mut view = Self {
            backend,
            state: WorkspaceState::default(),
            screen: Screen::default(),
            projects: vec![],
            recent: vec![],
            removing: None,
            removing_folder: None,
            delete_name_input,
            open_tools: HashSet::new(),
            chips: HashMap::new(),
            sent_references: HashMap::new(),
            status: None,
            composer_before: String::new(),
            slash_typed: None,
            slash_quiet: false,
            slash_skills: Vec::new(),
            status_tick: 0,
            turn_tokens: Vec::new(),
            provenance_open: HashMap::new(),
            artifact_open: None,
            artifact_versions: HashMap::new(),
            copied: None,
            message_menu: None,
            palette: PaletteState::default(),
            palette_input,
            find: Default::default(),
            find_input,
            aside_input,
            plan_feedback,
            asides: HashMap::new(),
            aside_run: None,
            aside_quotes: Vec::new(),
            capabilities: None,
            capabilities_error: None,
            archive: None,
            archive_error: None,
            merge: None,
            merge_error: None,
            merge_summary,
            imports: Default::default(),
            webview_imports: Default::default(),
            archive_fields,
            model_view: None,
            model_view_error: None,
            model_view_open: HashSet::new(),
            model_view_kind: None,
            compact_kind: Default::default(),
            compacting: false,
            compact_error: None,
            compact_resume: None,
            paused_failures: HashSet::new(),
            handoff: None,
            handoff_error: None,
            handoff_fields,
            reuse_selection: None,
            reuse_loading: false,
            reuse: None,
            reuse_error: None,
            reuse_fields,
            review: None,
            review_turn: None,
            trajectory: None,
            trajectory_error: None,
            trajectory_open: HashSet::new(),
            calendar_month: None,
            calendar: None,
            calendar_request: 0,
            calendar_day: None,
            calendar_hidden: HashSet::new(),
            journey_month: None,
            journey: None,
            journey_request: 0,
            journey_search,
            recap_drafting: None,
            journey_graph_shown: false,
            journey_graph: None,
            journey_graph_drawn: false,
            journey_edge: None,
            journey_output: None,
            journey_source: None,
            journey_run: None,
            assistant_plan: Vec::new(),
            memory_tidy: None,
            console_runtime: None,
            console_objects: None,
            console_entries: Vec::new(),
            console_input,
            console_busy: false,
            console_scroll: ScrollHandle::new(),
            mandates: None,
            mandate_editing: None,
            mandate_project: None,
            mandate_interval: 86_400,
            mandate_report_interval: 7 * 86_400,
            mandate_review: true,
            mandate_fields,
            mandate_deleting: None,
            mandate_replying: None,
            daily_recap: None,
            recap_time,
            schedules: None,
            schedule_fields,
            schedule_project: None,
            schedule_repeat: crate::schedules::Repeat::Daily,
            schedule_weekday: 0,
            schedule_deleting: None,
            schedule_note: None,
            papers: Vec::new(),
            paper: None,
            paper_request: 0,
            paper_renaming: false,
            paper_fields,
            paper_item_fields,
            paper_item_kind: wisp_server::desktop::PaperItemKind::Figure,
            paper_item_parent: None,
            paper_item_editing: None,
            paper_source_item: None,
            paper_source_kind: wisp_server::desktop::PaperSourceKind::Result,
            paper_source_fields,
            paper_source_selection: Default::default(),
            paper_source_audience: Default::default(),
            paper_anchor_kind: "message_span",
            paper_anchor_fields,
            paper_sources_open: Default::default(),
            paper_choices: Vec::new(),
            paper_choices_more: false,
            paper_passage: None,
            paper_passage_field,
            paper_rerunning: None,
            paper_capsule_saving: false,
            paper_policy: Default::default(),
            paper_check: None,
            paper_checking: false,
            paper_waiving: None,
            paper_waiver_field,
            journal_fields,
            recap_editing: None,
            recap_fields,
            journal_category: "progress",
            file_naming: None,
            file_name_input,
            file_search,
            file_trashing: None,
            file_problem: None,
            file_notice: None,
            file_as_text: false,
            palette_scroll: ScrollHandle::new(),
            sessions: vec![],
            inbox: vec![],
            known: HashMap::new(),
            tabs: Default::default(),
            centre: Default::default(),
            centre_menu: None,
            centre_width: std::rc::Rc::default(),
            panel_width: std::rc::Rc::default(),
            remote: None,
            run_logs: HashMap::new(),
            open_search: None,
            search_details: None,
            search_busy: false,
            search_problem: None,
            search_output: None,
            file_origin: None,
            pinned: vec![],
            show_archived: false,
            list_request: 0,
            paging: organise::Paging::default(),
            notices: crate::notices::Notices::default(),
            notice_timer: None,
            arrangement: Arrangement::default(),
            folders: vec![],
            grouped: HashMap::new(),
            branched: HashMap::new(),
            dispatched: HashMap::new(),
            timer: None,
            timer_busy: false,
            rewind_turn: None,
            collapsed: HashSet::new(),
            picks: Picks::default(),
            folder_target: None,
            folder_name,
            transfer_copy: false,
            transfer_artifacts: false,
            transfer_preview: None,
            sharing: None,
            notice: None,
            memos: None,
            memos_request: 0,
            memo_target: None,
            memo_title,
            memo_body,
            memo_search,
            memory: None,
            memory_request: 0,
            memory_target: None,
            memory_suggesting: false,
            memory_suggested: None,
            memory_text,
            appearance: Default::default(),
            look: Default::default(),
            font_fields,
            language: Default::default(),
            notify: Default::default(),
            notify_tone: crate::notify::sound_of(crate::notify::System::current(), None),
            update: None,
            update_at_start: true,
            update_banner: None,
            launch: Default::default(),
            send_key: Default::default(),
            sidebar_hidden: false,
            settings_section: Default::default(),
            turn_defaults: None,
            failure_analysis: None,
            review_model: String::new(),
            subagent_model: String::new(),
            picture_model: String::new(),
            steps_input,
            grants: vec![],
            tool_paths: vec![],
            tool_paths_saved: false,
            global_grants: vec![],
            approvals: None,
            settings_error: None,
            skills: None,
            skills_search,
            skill_tags_editing: None,
            skill_tags_input,
            settings_search,
            connectors: None,
            connectors_search,
            open_connector: None,
            environments: None,
            toolchain: None,
            toolchain_choice: vec!["python"],
            toolchain_following: false,
            onboarding_step: 0,
            onboarding_key,
            onboarding_saving: false,
            onboarding_error: None,
            network: None,
            network_fields,
            network_saved: false,
            env_target: None,
            env_checking: Default::default(),
            project_form: None,
            project_fields,
            project_form_ready: false,
            project_form_error: None,
            env_fields,
            env_password_sign_in: false,
            env_password,
            env_has_password: false,
            quick_actions: Vec::new(),
            quick_editing: None,
            quick_fields,
            quick_workflow: String::new(),
            workspace_choice,
            separate_storage: None,
            privacy: Default::default(),
            privacy_picks: HashSet::new(),
            workspace_saved: false,
            command_hooks: vec![],
            project_hook_file: None,
            hook_editing: None,
            hook_fields,
            decision: None,
            decision_draft: Default::default(),
            decision_fields,
            decision_note: None,
            decision_testing: false,
            browser: None,
            browser_fields,
            browser_prompts: Default::default(),
            browser_project: None,
            browser_watch: None,
            workflows: Vec::new(),
            workflow_editing: None,
            workflow_fields,
            workflow_request,
            workflow_skills: Vec::new(),
            workflow_skill_picks: Vec::new(),
            workflow_converting: false,
            terminal: terminal_view::TerminalDock::new(cx),
            workflow_tasks: Vec::new(),
            workflow_approval: Default::default(),
            workflow_pick: None,
            workflow_drag: None,
            remote_access: None,
            remote_qr_shown: false,
            remote_fields,
            remote_watch: 0,
            device: None,
            device_fields,
            device_token: None,
            device_note: None,
            plugins: None,
            plugin_installing: false,
            community_skills: None,
            skill_candidates: vec![],
            skill_store_busy: false,
            skill_store_note: None,
            github_skill_fields,
            plugin_address,
            plugin_removing: None,
            pet: None,
            pet_folder,
            pet_view,
            experts: Vec::new(),
            expert_editing: None,
            expert_model: String::new(),
            expert_skills: None,
            expert_connectors: None,
            expert_fields,
            acp_agents: None,
            acp_editing: None,
            acp_fields,
            acp_testing: None,
            acp_signed: Default::default(),
            acp_setting_open: None,
            subscriptions: Vec::new(),
            sign_in: None,
            sign_in_state: None,
            sign_in_redirect,
            sign_in_note: None,
            codex_accounts: Vec::new(),
            codex_usage: std::collections::HashMap::new(),
            codex_usage_checking: false,
            codex_removing: None,
            weixin: Default::default(),
            assistant_weixin: Default::default(),
            feishu: None,
            feishu_fields,
            feishu_lark: false,
            feishu_qr: None,
            feishu_bind_id: 0,
            feishu_note: None,
            lab: None,
            lab_fields,
            lab_invite: None,
            lab_removing: None,
            lab_busy: false,
            lab_note: None,
            project_sync: None,
            project_sync_busy: false,
            project_sync_note: None,
            project_sync_conflict: false,
            project_sync_replace: None,
            sync_settings: None,
            sync_fields,
            sync_backend: "relay".into(),
            sync_note: None,
            join_code,
            join_busy: false,
            join_note: None,
            new_project_name,
            new_project_description,
            new_project_layout: true,
            new_project_note: None,
            explorations: Vec::new(),
            batches: Vec::new(),
            batches_for: None,
            agent_cards: HashMap::new(),
            run_card_rows: HashMap::new(),
            own_run_cards: HashMap::new(),
            dismissed_runs: HashSet::new(),
            backgrounded_runs: HashSet::new(),
            swarm: None,
            batches_request: 0,
            budget_target: None,
            budget_input,
            budget_changes: HashMap::new(),
            explore_name,
            explore_from: None,
            explore_preview: None,
            explore_question: None,
            explore_busy: false,
            explore_note: None,
            acp_reports: Default::default(),
            mcp_servers: None,
            mcp_editing: None,
            mcp_http: true,
            mcp_name,
            mcp_target,
            mcp_token,
            mcp_pairs,
            mcp_route,
            mcp_tested: HashMap::new(),
            credentials: vec![],
            custom_credentials: vec![],
            custom_credential_fields,
            usage: None,
            storage: None,
            saved: vec![],
            saved_request: 0,
            favorites: None,
            favorites_request: 0,
            favorites_filter: Default::default(),
            favorites_search,
            favorite: None,
            favorite_image: None,
            favorite_versions: Vec::new(),
            favorite_shown: 0,
            favorite_code,
            favorite_edit_base: String::new(),
            file_image: None,
            file_document: None,
            file_pages: None,
            file_sequences: None,
            sequence_page: 0,
            file_structure: None,
            structure_turn: (0.0, 0.0),
            structure_drag: None,
            picture: crate::picture::View::default(),
            picture_geometry: std::rc::Rc::default(),
            picture_pan: None,
            picture_mark: None,
            picture_pins: crate::picture::Pins::default(),
            pin_note,
            picture_cutting: false,
            inline_images: HashMap::new(),
            reply_artifacts: Vec::new(),
            generated: HashMap::new(),
            generated_open: HashSet::new(),
            reply_artifacts_request: 0,
            favorite_request: 0,
            rename,
            settings,
            composer,
            search,
            rows: vec![],
            history: vec![],
            earlier_users: 0,
            before: None,
            history_under: (None, 0),
            history_epoch: 0,
            list: ListState::new(0, ListAlignment::Bottom, px(300.)),
            demos: vec![],
            demo: None,
            demo_list: ListState::new(0, ListAlignment::Top, px(300.)),
            demo_open: HashSet::new(),
            demo_note: None,
            layers: EscapeStack::default(),
            files: FilesState::default(),
            file_view,
            remote_text,
            remote_image: None,
            file_edit: None,
            file_drafts: Default::default(),
            close_confirmed: false,
            expanded_run: None,
            panel_tab: PanelTab::default(),
            kept_tabs: PanelTab::ALL.to_vec(),
            run_cleaning: None,
            run_review: None,
            run_review_filter,
            transfers_folded: false,
            conversation_hosts: None,
            hosts_busy: false,
            storage_host: None,
            host_detail: None,
            staged_removing: None,
            storage_fields,
            interpreter_fields,
            artifacts: vec![],
            artifacts_all: false,
            artifacts_by_folder: false,
            artifact_origin: None,
            artifacts_request: 0,
            ready: false,
            pending: false,
            loading_history: false,
            error: None,
            _subscriptions: subscriptions,
            poll: None,
            watching: false,
            watch: None,
        };
        view.initialize(window, cx);
        view
    }

    /// Closes the topmost open layer. Returns false when nothing was open, so
    /// the key continues to whatever else handles Escape.
    fn dismiss_top(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.layers.dismiss_top() {
            Some(Layer::FilePreview) => {
                // An unsaved edit stays as the file's draft.
                self.file_edit = None;
                self.files.close_preview();
                self.file_image = None;
                self.file_document = None;
                self.file_pages = None;
                self.restore_focus(window, cx);
                // A file revealed from the palette has no listing under it yet.
                self.ensure_files(window, cx);
            }
            Some(Layer::FilesPanel) => self.restore_focus(window, cx),
            Some(Layer::PanelTabMenu | Layer::StartMenu | Layer::FilesMenu) => {}
            Some(Layer::MessageMenu) => self.message_menu = None,
            Some(Layer::Inbox) => {}
            Some(Layer::ConfirmCleanRun) => self.run_cleaning = None,
            Some(Layer::HostStorageForm) => self.storage_host = None,
            Some(Layer::RemotePreview) => self.close_remote_look(),
            Some(Layer::HostDetail) => {
                self.host_detail = None;
                self.staged_removing = None;
            }
            Some(Layer::ConfirmRemoveStaged) => self.staged_removing = None,
            Some(Layer::RunReview) => self.run_review = None,
            Some(Layer::PrivacyMode) => self.privacy_picks.clear(),
            Some(Layer::ConfirmRunFilesDelete) => {
                if let Some(review) = &mut self.run_review {
                    review.asking = None;
                }
            }
            // These layers own the focused field, which goes away with them.
            Some(Layer::Settings | Layer::Rename | Layer::ConfirmDelete) => {
                self.focus_composer(window, cx)
            }
            Some(Layer::ConfirmRemoveProject) => {
                self.removing = None;
                self.removing_folder = None;
            }
            Some(
                Layer::Palette
                | Layer::ConfirmFullAccess
                | Layer::EffortMenu
                | Layer::ModelMenu
                | Layer::RunsOnMenu
                | Layer::ExpertMenu
                | Layer::AgentMenu
                | Layer::QuickMenu
                | Layer::PlusMenu,
            ) => self.focus_composer(window, cx),
            Some(Layer::SlashMenu) => {
                // Shut while a word follows the `/`, it stays shut.
                if self.slash_typed.take().is_some() {
                    self.slash_quiet = true;
                }
                self.focus_composer(window, cx)
            }
            Some(Layer::AcpSettingMenu) => {
                self.acp_setting_open = None;
                self.focus_composer(window, cx)
            }
            // Settings stays open underneath and keeps its focus.
            Some(Layer::ConfirmDeleteModel) => {}
            Some(Layer::SelectMode) => self.picks.clear(),
            Some(Layer::FolderName) => {
                self.folder_target = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::FolderMenu | Layer::ConfirmDeleteFolder) => self.folder_target = None,
            Some(Layer::ConfirmRewind) => self.rewind_turn = None,
            Some(Layer::MemoEditor) => {
                self.memo_target = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::ConfirmDeleteMemo) => self.memo_target = None,
            Some(Layer::MemoryEditor) => {
                self.memory_target = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::ConfirmDeleteMemory) => self.memory_target = None,
            // Nothing was deleted; the Memory tab stays as it was.
            Some(Layer::ConfirmClearMemory) => {}
            Some(Layer::ShareMessages) => self.sharing = None,
            Some(Layer::Favorites) => {
                self.favorite = None;
                self.favorite_image = None;
                self.favorite_versions.clear();
                self.focus_composer(window, cx);
            }
            // The edit is given up; the version shown stays.
            Some(Layer::FavoriteCode) => {}
            // The page stays open underneath and keeps its focus.
            Some(Layer::ConfirmRemoveFavorite) => {}
            Some(Layer::McpForm | Layer::ConfirmRemoveMcp) => self.mcp_editing = None,
            Some(Layer::ExpertForm | Layer::ConfirmRemoveExpert) => self.expert_editing = None,
            Some(Layer::AcpAgentForm | Layer::ConfirmRemoveAcpAgent) => self.acp_editing = None,
            Some(Layer::ConfirmRemoveCodexAccount) => self.codex_removing = None,
            Some(
                Layer::ConfirmUnbindWeixin
                | Layer::ConfirmUnbindAssistantWeixin
                | Layer::ConfirmUnbindFeishu
                | Layer::ConfirmLeaveLab,
            ) => {}
            Some(Layer::ConfirmRemoveLabMember) => self.lab_removing = None,
            Some(Layer::ConfirmRemovePlugin) => self.plugin_removing = None,
            Some(Layer::ConfirmResetRemoteCode) => {}
            Some(Layer::WorkflowForm) => {
                self.workflow_editing = None;
                self.workflow_tasks.clear();
            }
            Some(Layer::ConfirmRemoveWorkflow) => self.workflow_editing = None,
            Some(Layer::WorkflowFromSkills) => self.workflow_skill_picks.clear(),
            Some(Layer::TerminalHosts) => {}
            Some(Layer::SkillTags) => self.skill_tags_editing = None,
            Some(Layer::HookForm) => self.hook_editing = None,
            Some(Layer::PluginAddress | Layer::SkillFromGithub) => {}
            Some(Layer::QuickActionForm | Layer::ConfirmRemoveQuickAction) => {
                self.quick_editing = None
            }
            Some(Layer::EnvironmentForm | Layer::ConfirmRemoveEnvironment) => {
                self.env_target = None
            }
            Some(Layer::ProjectSettings) => {
                self.project_form = None;
                self.project_form_error = None;
            }
            Some(Layer::ConfirmSyncReplace | Layer::JoinSyncedProject | Layer::NewProject) => {}
            Some(Layer::Examples) => self.close_examples(),
            Some(Layer::ExampleCopy) => {}
            Some(Layer::StartExploration) => self.explore_from = None,
            Some(Layer::ExplorationPreview) => self.explore_preview = None,
            Some(Layer::ConfirmExploration) => self.explore_question = None,
            Some(Layer::Find) => {
                self.find.clear();
                self.focus_composer(window, cx);
            }
            Some(Layer::FileName) => {
                self.file_naming = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::Outline) => self.focus_composer(window, cx),
            Some(Layer::Journey) => {
                self.journey = None;
                self.forget_journey_extras();
                self.focus_composer(window, cx);
            }
            Some(Layer::Calendar) => {
                self.calendar = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::Capabilities) => {
                self.capabilities = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::Trajectory) => {
                self.clear_trajectory();
                self.focus_composer(window, cx);
            }
            Some(Layer::Review) => {
                self.review = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::ModelView) => {
                self.clear_model_view();
                self.focus_composer(window, cx);
            }
            Some(Layer::CompactContext) => {
                self.compact_error = None;
                self.compact_resume = None;
                if !self.layers.contains(Layer::ModelView) {
                    self.focus_composer(window, cx);
                }
            }
            Some(Layer::Handoff) => {
                self.handoff = None;
                self.handoff_error = None;
                if !self.layers.contains(Layer::ModelView) {
                    self.focus_composer(window, cx);
                }
            }
            // The range stays chosen under the dialog, as in the WebView.
            Some(Layer::SessionReuse) => {
                self.reuse = None;
                self.reuse_error = None;
            }
            Some(Layer::ReuseRange) => {
                self.reuse_selection = None;
                self.reuse_loading = false;
                self.reuse = None;
                self.reuse_error = None;
            }
            // A draft with changes asks before they are given up.
            Some(Layer::ResearchArchive) => {
                if self.archive_needs_asking(cx) {
                    self.layers.open(Layer::ResearchArchive);
                    self.ask_before_leaving_archive(archive::Leaving::Close, cx);
                } else {
                    self.archive = None;
                    self.archive_error = None;
                    self.focus_composer(window, cx);
                }
            }
            Some(Layer::ConfirmDiscardArchive) => self.stop_asking_about_archive(),
            Some(Layer::BranchMerge) => {
                self.merge = None;
                self.merge_error = None;
                self.focus_composer(window, cx);
            }
            // Putting the guide away is finishing it, as in the WebView.
            Some(Layer::Onboarding) => self.finish_onboarding(cx),
            Some(
                layer @ (Layer::BrowserTabCleanup
                | Layer::BrowserNeedsHuman
                | Layer::BrowserExtensionUpdate),
            ) => self.escape_browser_prompt(layer, window, cx),
            // The newest card goes; the others wait for the next press.
            Some(Layer::SessionNotices) => {
                self.notices.dismiss_newest();
                self.sync_notices(window, cx);
            }
            Some(Layer::ImportConversations) => {
                self.imports = Default::default();
            }
            Some(Layer::WebViewImport) => self.webview_imports = Default::default(),
            // The page stays open underneath.
            Some(Layer::JournalForm) => {}
            Some(Layer::RecapForm) => self.recap_editing = None,
            Some(Layer::JourneyEdge) => self.journey_edge = None,
            Some(Layer::JourneyRun) => self.journey_run = None,
            Some(Layer::Schedules) => self.focus_composer(window, cx),
            Some(Layer::ScheduleForm) => {}
            Some(Layer::ConfirmDeleteSchedule) => self.schedule_deleting = None,
            Some(Layer::RuntimeConsole) => {
                self.console_runtime = None;
                self.focus_composer(window, cx);
            }
            Some(Layer::TaskBudget) => self.budget_target = None,
            Some(Layer::Mandates) => self.focus_composer(window, cx),
            Some(Layer::MandateForm) => {}
            Some(Layer::ConfirmDeleteMandate) => self.mandate_deleting = None,
            Some(Layer::Papers) => {
                self.paper = None;
                self.paper_check = None;
                self.paper_waiving = None;
                self.focus_composer(window, cx);
            }
            Some(
                Layer::PaperForm
                | Layer::PaperItemForm
                | Layer::PaperSources
                | Layer::PaperAnchor
                | Layer::ConfirmRemovePaper,
            ) => {}
            Some(Layer::PaperFinalize) => {
                self.paper_check = None;
                self.paper_waiving = None;
            }
            Some(Layer::PaperWaiver) => self.paper_waiving = None,
            Some(Layer::ConfirmTrashFile) => self.file_trashing = None,
            // The edit stays as it was, unsaved.
            Some(Layer::FileChangedOnDisk | Layer::ConfirmCloseUnsaved) => {}
            // The region and its note are given up; marking goes on.
            Some(Layer::PictureRegion) => self.drop_picture_mark(window, cx),
            Some(Layer::PictureSelect) => self.picture_mark = None,
            // Menus and questions that hold no focus of their own.
            Some(
                Layer::SortMenu
                | Layer::ListMenu
                | Layer::BatchGroupMenu
                | Layer::ConfirmBatchDelete
                | Layer::GroupMenu
                | Layer::ConversationMenu,
            ) => {}
            // A move waiting to be confirmed is forgotten with its menu.
            Some(Layer::TransferMenu) => self.transfer_preview = None,
            Some(Layer::CentreTabMenu) => self.centre_menu = None,
            None => return false,
        }
        cx.notify();
        true
    }

    /// Runs a window-wide shortcut. Returns false when it does not apply
    /// here, so the key reaches whatever else is listening.
    fn run_shortcut(
        &mut self,
        command: Shortcut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.ready {
            return false;
        }
        match command {
            Shortcut::Settings => {
                self.layers.close(Layer::Palette);
                if !self.layers.contains(Layer::Settings) {
                    self.open_settings(window, cx);
                }
                true
            }
            Shortcut::Palette => {
                self.toggle_palette(window, cx);
                true
            }
            Shortcut::Commands => {
                self.toggle_command_palette(window, cx);
                true
            }
            Shortcut::Find => {
                // Only where a conversation is what is showing.
                if self.screen != Screen::Project
                    || self.state.session.is_none()
                    || self.layers.contains(Layer::Settings)
                    || self.layers.contains(Layer::Calendar)
                    || self.layers.contains(Layer::Journey)
                    || self.layers.contains(Layer::Papers)
                    || self.layers.contains(Layer::Schedules)
                    || self.layers.contains(Layer::Mandates)
                    || self.layers.contains(Layer::Favorites)
                    || self.layers.contains(Layer::Palette)
                    || self.layers.contains(Layer::ProjectSettings)
                    || self.layers.contains(Layer::JoinSyncedProject)
                    || self.layers.contains(Layer::NewProject)
                    || self.layers.contains(Layer::Examples)
                    || self.layers.contains(Layer::StartExploration)
                    || self.layers.contains(Layer::ExplorationPreview)
                {
                    return false;
                }
                self.open_find(window, cx);
                true
            }
            Shortcut::NewConversation => {
                if self.screen != Screen::Project
                    || self.layers.contains(Layer::Settings)
                    || self.layers.contains(Layer::Calendar)
                    || self.layers.contains(Layer::Journey)
                    || self.layers.contains(Layer::Papers)
                    || self.layers.contains(Layer::Schedules)
                    || self.layers.contains(Layer::Mandates)
                    || self.layers.contains(Layer::Favorites)
                    || self.layers.contains(Layer::Palette)
                {
                    return false;
                }
                self.create_session(window, cx);
                true
            }
            Shortcut::Privacy => {
                self.toggle_privacy(window, cx);
                true
            }
            Shortcut::Sidebar => {
                // The sidebar belongs to a project's page.
                if self.screen != Screen::Project || self.layers.contains(Layer::Settings) {
                    return false;
                }
                self.toggle_sidebar(window, cx);
                true
            }
            Shortcut::SearchHistory => {
                // The sidebar's search reads titles and messages alike.
                if self.screen != Screen::Project || self.layers.contains(Layer::Settings) {
                    return false;
                }
                self.layers.close(Layer::Palette);
                if self.sidebar_hidden {
                    self.toggle_sidebar(window, cx);
                }
                self.search.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
                true
            }
        }
    }

    /// A closed layer takes its focused element with it; keep typing possible.
    fn restore_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_view.read(cx).focus_handle(cx).is_focused(window) {
            self.focus_composer(window, cx);
        }
    }

    /// Words the empty text fields again, after the language changed: a
    /// field keeps the placeholder it was made with otherwise.
    pub(super) fn word_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composer = self.send_key.placeholder();
        let fields = [
            (&self.composer, composer),
            (&self.search, tr("Search conversations…")),
            (&self.rename, tr("Conversation title")),
            (&self.folder_name, tr("Group name")),
            (&self.delete_name_input, tr("Type the project's name")),
            (&self.memo_title, tr("Title (optional)")),
            (
                &self.memo_body,
                tr("A prompt you reuse, a server address, a reminder…"),
            ),
            (&self.memo_search, tr("Search memos")),
            (&self.mcp_name, tr("Lab notebook")),
            (
                &self.project_fields[2],
                tr("Always report effect sizes with confidence intervals."),
            ),
            (&self.project_fields[3], tr("Off")),
            (&self.project_fields[4], tr("Off")),
            (&self.project_fields[5], tr("Off")),
            (&self.find_input, tr("Find in this conversation…")),
            (&self.aside_input, tr("Ask about this conversation…")),
            (
                &self.plan_feedback,
                tr("Tell Wisp what to change in this plan."),
            ),
            (&self.journey_search, tr("Filter this month…")),
            (&self.journal_fields[0], tr("The reads are clean")),
            (&self.skills_search, tr("Find a skill…")),
            (&self.settings_search, tr("Search settings…")),
            (&self.connectors_search, tr("Find a connector or a tool…")),
            (&self.favorites_search, tr("Search your favorites…")),
        ];
        for (field, text) in fields {
            field.update(cx, |input, cx| input.set_placeholder(text, window, cx));
        }
    }

    /// Opens the system's terminal in the folder being browsed, or in the
    /// project's own when the Files tab is at its root.
    pub(super) fn open_terminal(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self
            .state
            .project
            .as_ref()
            .and_then(|id| self.projects.iter().find(|project| &project.id == id))
            .map(|project| std::path::PathBuf::from(&project.workspace_dir))
        else {
            return;
        };
        // The path browsed is the project's own listing: relative, and
        // without a way out of the folder.
        let browsed = std::path::Path::new(&self.files.path);
        let inside = browsed
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)));
        let folder = if inside { root.join(browsed) } else { root };
        self.error = crate::terminal::open(&folder).err();
        cx.notify();
    }

    /// Opens a terminal in the dock on an execution context (an SSH host,
    /// in `folder` there when given), as many times as asked.
    pub(super) fn open_host_terminal(
        &mut self,
        context: String,
        folder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_terminal_in(Some(context), folder, window, cx);
    }

    /// Opens a terminal in the dock in the folder the Files tab is at, or
    /// the project's own when it is at its root.
    pub(super) fn open_terminal_here(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let browsed = std::path::Path::new(&self.files.path);
        let inside = browsed
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)));
        let folder = (inside && !self.files.path.is_empty()).then(|| self.files.path.clone());
        self.open_terminal_in(None, folder, window, cx);
    }

    /// Tells a user who is not looking at the window that the open
    /// conversation needs them, in the way they chose.
    fn tell_user(&self, event: crate::notify::Event, window: &Window) {
        if !crate::notify::wanted(self.notify, window.is_window_active()) {
            return;
        }
        let title = self
            .current_row()
            .map(|row| present::one_line(&row.title))
            .unwrap_or_default();
        crate::notify::post(self.notify, self.notify_tone, event, &title);
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Changes the selection. File state belongs to a project, so a preview
    /// from the previous project must not stay open over the new one.
    fn choose(&mut self, project: Option<String>, session: Option<String>) {
        // The document the centre shows is the conversation's being left:
        // its tab stays with it, and it is read again when it comes back
        // (`restore_centre`).
        if self.state.session != session && self.preview_in_centre() {
            self.drop_preview();
        }
        if self.state.project != project {
            self.layers.close(Layer::FilePreview);
            // A host's runs and placed files are the project's.
            self.host_detail = None;
            self.staged_removing = None;
            self.layers.close(Layer::HostDetail);
            // The list, its groups and the picks belong to the project.
            self.layers.close(Layer::SelectMode);
            for layer in Layer::SIDEBAR {
                self.layers.close(layer);
            }
            self.picks.clear();
            self.paging = organise::Paging::default();
            self.folders.clear();
            self.grouped.clear();
            self.branched.clear();
            self.folder_target = None;
            self.notice = None;
            // Memos are the project's, and so is an edit in progress.
            self.layers.close(Layer::MemoEditor);
            self.layers.close(Layer::ConfirmDeleteMemo);
            self.memos = None;
            self.memo_target = None;
            self.layers.close(Layer::MemoryEditor);
            self.layers.close(Layer::ConfirmDeleteMemory);
            self.layers.close(Layer::ConfirmClearMemory);
            self.memory = None;
            self.memory_target = None;
        }
        // A note about the conversation being left is not about the next.
        if self.state.session != session {
            self.notice = None;
        }
        // These act on the conversation that is being left.
        for layer in [
            Layer::Rename,
            Layer::ConfirmDelete,
            Layer::ConversationMenu,
            Layer::GroupMenu,
            Layer::TransferMenu,
            Layer::ConfirmRewind,
            Layer::ConfirmFullAccess,
            Layer::EffortMenu,
            Layer::ModelMenu,
            Layer::RunsOnMenu,
            Layer::ExpertMenu,
            Layer::AgentMenu,
            Layer::AcpSettingMenu,
            Layer::QuickMenu,
            Layer::PlusMenu,
            Layer::SlashMenu,
            Layer::MessageMenu,
            Layer::Find,
            Layer::Outline,
            Layer::Trajectory,
            Layer::Review,
            Layer::ModelView,
            Layer::CompactContext,
            Layer::Handoff,
            Layer::ResearchArchive,
            Layer::BranchMerge,
            Layer::RunReview,
            Layer::ShareMessages,
            Layer::CentreTabMenu,
        ] {
            self.layers.close(layer);
        }
        self.centre_menu = None;
        self.run_review = None;
        self.sharing = None;
        self.archive = None;
        self.merge = None;
        self.clear_trajectory();
        self.clear_model_view();
        self.compact_error = None;
        self.compact_resume = None;
        self.handoff = None;
        self.handoff_error = None;
        self.review = None;
        self.find.clear();
        if self.state.session != session {
            self.saved.clear();
            self.rewind_turn = None;
            self.status = None;
            self.conversation_hosts = None;
            self.copied = None;
            // The list belonged to the conversation, or project, being left.
            self.artifacts.clear();
            // A range of turns is chosen in one conversation; its dialog
            // goes with it.
            self.layers.close(Layer::ReuseRange);
            self.reuse_selection = None;
            self.reuse_loading = false;
            self.reuse = None;
            self.reuse_error = None;
        }
        if self.state.project != project {
            // Paths are the project's own; another project's mean others.
            self.inline_images.clear();
        }
        self.files.set_project(project.as_deref());
        self.state.select(project, session);
    }

    fn initialize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = true;
        self.error = None;
        let reply = self.backend.initialize();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(()) => {
                        this.ready = true;
                        // Turns the host starts by itself show up too.
                        this.watch_activity(window, cx);
                        this.load_projects(window, cx);
                        this.load_recent(window, cx);
                        this.load_inbox(window, cx);
                        this.load_model_settings(true, window, cx);
                        this.check_onboarding(window, cx);
                        this.load_arrangement(window, cx);
                        this.load_general(window, cx);
                        // The composer offers the saved hosts.
                        this.load_environments(window, cx);
                        // The desk display can ask for a conversation.
                        this.follow_device_focus(window, cx);
                        // What the browser asks of the user, and its update.
                        this.watch_browser(window, cx);
                        // A Run moved to the background is told about
                        // when it ends.
                        this.listen_for_run_endings(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Has the app tell the user, once whatever window asks first, each
    /// time a Run moved to the background ends, as the WebView does.
    fn listen_for_run_endings(&self, cx: &mut Context<Self>) {
        /// Set once the app listens.
        struct Listening;
        impl Global for Listening {}
        if cx.has_global::<Listening>() {
            return;
        }
        let Some(mut endings) = self.backend.background_run_endings() else {
            return;
        };
        cx.set_global(Listening);
        let app: &App = cx;
        app.spawn(async move |cx| {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match endings.recv().await {
                    Ok(ended) => {
                        if cx.update(|cx| tell_run_ended(&ended, cx)).is_err() {
                            return;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                }
            }
        })
        .detach();
    }

    /// Whether `session` is the conversation open here.
    fn shows(&self, session: &str) -> bool {
        self.state.session.as_deref() == Some(session)
    }

    /// Opens each conversation the StickS3 desk display asks for, and
    /// brings the window forward.
    fn follow_device_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            while let Ok((project, session)) = receive(backend.next_device_focus()).await {
                let open = this.update_in(cx, |this, window, cx| {
                    window.activate_window();
                    this.open_session(project, session, window, cx);
                    cx.notify();
                });
                if open.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn load_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self
            .backend
            .request::<Vec<ProjectSummary>>("list_projects", None, json!({}));
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(projects) => this.projects = home::ordered(projects),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.recent_activity(12);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((recent, finished)) => {
                        // Conversations that are not open and stopped
                        // working are told about like the open one.
                        let open = this.state.session.clone();
                        for (event, title) in
                            crate::notify::elsewhere(&this.recent, &recent, open.as_deref())
                        {
                            if crate::notify::wanted(this.notify, window.is_window_active()) {
                                crate::notify::post(this.notify, this.notify_tone, event, &title);
                            }
                        }
                        // Those of other projects whose turn ended get a
                        // card in the corner too.
                        let stopped =
                            crate::notify::stopped(&this.recent, &recent, open.as_deref());
                        let here = this.shown_project();
                        let arrived = crate::notices::arrivals(
                            &stopped,
                            &finished,
                            here.as_deref(),
                            open.as_deref(),
                            |project| home::concealed(&this.privacy, project),
                        );
                        if !arrived.is_empty() {
                            let now = std::time::Instant::now();
                            for (row, failed) in arrived {
                                this.notices.push(row, failed, now);
                            }
                            this.sync_notices(window, cx);
                        }
                        this.recent = recent;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a project from the home page.
    pub(crate) fn enter_project(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A cloud-drive folder project takes in a newer version another
        // device saved, while this one has nothing unsaved.
        let adopting = self.backend.adopt_newer_folder_version(id.clone());
        cx.background_spawn(async move {
            let _ = receive(adopting).await;
        })
        .detach();
        self.screen = Screen::Project;
        self.layers.close(Layer::ConfirmRemoveProject);
        self.removing = None;
        self.removing_folder = None;
        self.select_project(id, window, cx);
        cx.notify();
    }

    /// Leaves the project for the home page. Turns keep running in the host.
    fn go_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state
            .save_draft(self.composer.read(cx).value().to_string());
        self.layers.close(Layer::FilesPanel);
        // What waits was opened from the project's bar.
        self.layers.close(Layer::Inbox);
        self.choose(None, None);
        self.sessions.clear();
        self.pinned.clear();
        self.show_archived = false;
        self.clear_transcript();
        for input in [&self.composer, &self.search] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.screen = Screen::Home;
        self.refresh_lists(window, cx);
        cx.notify();
    }

    fn toggle_star(
        &mut self,
        id: String,
        starred: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Shown at once; the reload below confirms or corrects it.
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
            project.starred = starred;
        }
        self.projects = home::ordered(std::mem::take(&mut self.projects));
        let reply = self.backend.star_project(id, starred);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.load_projects(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn ask_remove_project(&mut self, id: String, cx: &mut Context<Self>) {
        self.error = None;
        self.removing = Some(id);
        self.removing_folder = None;
        self.layers.open(Layer::ConfirmRemoveProject);
        cx.notify();
    }

    fn cancel_remove_project(&mut self, cx: &mut Context<Self>) {
        if self.layers.close(Layer::ConfirmRemoveProject) {
            self.removing = None;
            self.removing_folder = None;
            cx.notify();
        }
    }

    /// Turns the question into the stronger one: the folder goes too. The
    /// host says what is in it, or why it may not be deleted.
    fn ask_delete_project_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.removing.clone() else {
            return;
        };
        self.removing_folder = Some(home::FolderQuestion::Looking);
        self.delete_name_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let reply = self.backend.project_folder(id.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Cancelled, or asked about another project, meanwhile.
                if this.removing.as_ref() != Some(&id)
                    || this.removing_folder != Some(home::FolderQuestion::Looking)
                {
                    return;
                }
                this.removing_folder = Some(match result {
                    Ok(folder) => {
                        this.delete_name_input
                            .update(cx, |input, cx| input.focus(window, cx));
                        home::FolderQuestion::Ask(folder)
                    }
                    Err(reason) => home::FolderQuestion::Refused(reason),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Deletes the project and its folder, once its name has been typed.
    fn delete_project_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.layers.contains(Layer::ConfirmRemoveProject) {
            return;
        }
        let (Some(id), Some(home::FolderQuestion::Ask(folder))) =
            (self.removing.clone(), self.removing_folder.clone())
        else {
            return;
        };
        let typed = self.delete_name_input.read(cx).value().to_string();
        if !home::name_typed(&typed, &folder) {
            return;
        }
        self.pending = true;
        self.error = None;
        let reply = self.backend.delete_project_and_folder(id, typed);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                this.layers.close(Layer::ConfirmRemoveProject);
                let gone = this.removing.take();
                this.removing_folder = None;
                match result {
                    Ok(()) => {
                        if let Some(gone) = gone {
                            this.tabs.forget_project(&gone);
                        }
                        this.notice = Some(crate::i18n::fill(
                            tr("Deleted \"{}\" and its folder."),
                            &[&folder.name],
                        ));
                    }
                    Err(error) => this.error = Some(error),
                }
                this.refresh_lists(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.layers.contains(Layer::ConfirmRemoveProject) {
            return;
        }
        let Some(id) = self.removing.clone() else {
            return;
        };
        self.pending = true;
        self.error = None;
        let reply = self.backend.remove_project(id);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                this.layers.close(Layer::ConfirmRemoveProject);
                if let (Ok(()), Some(gone)) = (&result, this.removing.take()) {
                    this.tabs.forget_project(&gone);
                }
                this.removing = None;
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.refresh_lists(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Folds or unfolds one tool row and has the list measure it again.
    fn toggle_tool(&mut self, key: String, index: usize, cx: &mut Context<Self>) {
        if !self.open_tools.remove(&key) {
            self.open_tools.insert(key);
        }
        if index < self.rows.len() {
            self.list.splice(index..index + 1, 1);
        }
        cx.notify();
    }

    fn load_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.backend.request::<Vec<SessionSearchInfo>>(
            "native_conversation_inbox",
            None,
            json!({}),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(inbox) => {
                        // A card whose conversation was read somewhere goes.
                        let listed = inbox.iter().map(|row| row.id.as_str()).collect();
                        if this.notices.reconcile(&listed) {
                            this.sync_notices(window, cx);
                        }
                        this.inbox = inbox;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_lists(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_projects(window, cx);
        self.load_recent(window, cx);
        self.load_sessions(window, cx);
        self.load_inbox(window, cx);
    }

    fn select_project(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state
            .save_draft(self.composer.read(cx).value().to_string());
        self.choose(Some(id), None);
        self.sessions.clear();
        self.pinned.clear();
        self.show_archived = false;
        self.clear_transcript();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        // A name looked for in the project being left is not looked for here.
        self.file_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.load_sessions(window, cx);
    }

    fn clear_transcript(&mut self) {
        self.poll = None;
        self.rows.clear();
        // A side question about a run, or quoting a passage, was about the
        // conversation left, as the WebView clears its side chat's quotes.
        self.aside_run = None;
        self.aside_quotes.clear();
        self.turn_tokens.clear();
        self.batches.clear();
        self.batches_for = None;
        self.agent_cards.clear();
        self.run_card_rows.clear();
        self.own_run_cards.clear();
        self.backgrounded_runs.clear();
        self.swarm = None;
        self.drop_history();
        self.before = None;
        self.error = None;
        self.expanded_run = None;
        self.close_search();
        self.open_tools.clear();
        // The files under the answers, and what was open of them, were the
        // conversation's being left.
        self.reply_artifacts.clear();
        self.generated.clear();
        self.generated_open.clear();
        self.reply_artifacts_request += 1;
        self.list.reset(0);
    }

    fn load_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The round's candidates are listed with the conversations.
        self.load_explorations(window, cx);
        let selection = self.state.selection();
        let Some(project) = selection.project.clone() else {
            return;
        };
        let query = self.search.read(cx).value().to_string();
        let archived = self.show_archived;
        // Search results and the archive are for looking, not for landing in.
        let browsing = query.trim().is_empty() && !archived;
        self.list_request += 1;
        let request = self.list_request;
        // Every page loaded of this list is read again, so a refresh keeps
        // what "Load earlier" added.
        let key: organise::ListKey = (Some(project.clone()), query.clone(), archived);
        let pages = self.paging.pages_for(key.clone());
        let reply = self.backend.conversations(project, query, archived, pages);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) || this.list_request != request {
                    return;
                }
                match result {
                    Ok(list) => {
                        this.paging.answered(&key, pages, list.more);
                        for row in &list.rows {
                            this.known.insert(row.id.clone(), row.clone());
                        }
                        this.sessions = present::pinned_first(list.rows, &list.pinned);
                        this.pinned = list.pinned;
                        this.folders = list.folders;
                        this.grouped = list.grouped;
                        this.branched = list.branched;
                        this.dispatched = list.dispatched;
                        // A subagent's card follows its conversation's title
                        // and status.
                        this.place_agent_cards();
                        this.picks.keep(&this.sessions);
                        // The host may start a turn in the open conversation
                        // by itself (a subagent's report, a background batch's
                        // result, a scheduled task): follow it.
                        if present::unfollowed_turn(
                            this.state.session.as_deref(),
                            &this.sessions,
                            this.state.snapshot.as_ref().is_some_and(|s| s.running),
                        ) {
                            this.start_poll(window, cx);
                        }
                        if browsing && this.state.session.is_none() && !this.pending {
                            // The first one on screen, whatever the sections.
                            let shown = organise::visible(&this.session_rows(cx), &this.sessions);
                            if let Some(first) = shown.into_iter().next() {
                                this.select_session(first, window, cx);
                            }
                        }
                        this.mark_seen(window, cx);
                    }
                    Err(error) => {
                        this.paging.failed();
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Lists the page of conversations before the ones loaded, under them.
    pub(super) fn load_earlier(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.paging.earlier() {
            self.load_sessions(window, cx);
        }
        cx.notify();
    }

    fn select_session(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state
            .save_draft(self.composer.read(cx).value().to_string());
        if let Some(project) = self.state.project.clone() {
            self.tabs.visit(&project, &id);
            if let Some(row) = self.known.get(&id) {
                self.tabs.name(&id, &present::one_line(&row.title));
            }
        }
        // The conversation on screen has no card of its own.
        if self.notices.remove(&id) {
            self.sync_notices(window, cx);
        }
        self.choose(self.state.project.clone(), Some(id));
        self.clear_transcript();
        let draft = self.state.draft();
        self.fill_composer(draft, window, cx);
        self.start_poll(window, cx);
        self.load_status(window, cx);
        self.load_saved(window, cx);
        // The documents it had open come back, the one it showed with them.
        self.restore_centre(window, cx);
        self.ensure_files(window, cx);
        self.reload_artifacts(window, cx);
        self.load_reply_artifacts(window, cx);
        self.remember_place(window, cx);
    }

    /// Refreshes the Artifacts or Environments tab if that is what the
    /// panel is showing.
    fn reload_artifacts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layers.contains(Layer::FilesPanel) && self.panel_tab == PanelTab::Artifacts {
            self.load_artifacts(window, cx);
        }
        if self.layers.contains(Layer::FilesPanel) && self.panel_tab == PanelTab::Environments {
            self.load_conversation_hosts(window, cx);
        }
    }

    /// Opens a conversation from any project, as the inbox needs.
    pub(crate) fn open_session(
        &mut self,
        project: String,
        session: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.screen = Screen::Project;
        if self.state.project.as_ref() != Some(&project) {
            self.state
                .save_draft(self.composer.read(cx).value().to_string());
            self.choose(Some(project), None);
            self.sessions.clear();
            self.pinned.clear();
            self.show_archived = false;
            self.search
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.select_session(session, window, cx);
        self.load_sessions(window, cx);
    }

    /// Marks the open conversation read once its answer is actually on screen.
    fn mark_seen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.state.session.clone() else {
            return;
        };
        let Some(row) = self.known.get_mut(&id) else {
            return;
        };
        if !present::should_mark_seen(
            &row.status,
            self.state.snapshot.as_ref(),
            window.is_window_active(),
        ) {
            return;
        }
        // Applied locally first so repeated refreshes do not resend it.
        row.status = "complete".into();
        if let Some(row) = self.sessions.iter_mut().find(|row| row.id == id) {
            row.status = "complete".into();
        }
        self.inbox.retain(|row| row.id != id);
        let selection = self.state.selection();
        let reply = self.backend.request::<Value>(
            "native_conversation_seen",
            selection.project.clone(),
            json!({"session_id": id}),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(_) => {
                        this.load_projects(window, cx);
                        this.load_inbox(window, cx);
                    }
                    Err(error) if this.state.current(&selection) => this.error = Some(error),
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Keeps project, conversation and inbox status fresh, including turns
    /// in conversations that are not on screen: every two seconds while a
    /// turn runs anywhere, every ten otherwise, since the host may start one
    /// by itself (a scheduled task, a timer, a browser through remote
    /// access) and make a new conversation for it. Stops when the host
    /// cannot be read.
    fn watch_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.watching {
            return;
        }
        self.watching = true;
        let backend = self.backend.clone();
        self.watch = Some(cx.spawn_in(window, async move |this, cx| {
            let mut delay = Duration::from_secs(2);
            loop {
                cx.background_executor().timer(delay).await;
                let result = receive(backend.request::<Vec<ProjectSummary>>(
                    "list_projects",
                    None,
                    json!({}),
                ))
                .await;
                let busy = this
                    .update_in(cx, |this, window, cx| {
                        let busy = match result {
                            Ok(projects) => {
                                let busy = projects.iter().any(|p| p.running_count > 0);
                                this.projects = home::ordered(projects);
                                this.load_recent(window, cx);
                                this.load_sessions(window, cx);
                                this.load_inbox(window, cx);
                                Some(busy)
                            }
                            Err(error) => {
                                this.error = Some(error);
                                None
                            }
                        };
                        this.watching = busy.is_some();
                        cx.notify();
                        busy
                    })
                    .ok()
                    .flatten();
                delay = match busy {
                    Some(true) => Duration::from_secs(2),
                    Some(false) => Duration::from_secs(10),
                    None => break,
                };
            }
        }));
    }

    fn start_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        if selection.session.is_none() {
            return;
        }
        if self.agents_wanted() {
            self.load_batches(window, cx);
        }
        if selection
            .session
            .as_deref()
            .is_some_and(wisp_server::desktop::is_assistant)
        {
            self.load_assistant_plan(window, cx);
        }
        self.load_turn_tokens(window, cx);
        // A timer is kept by the host, so it shows after a restart too.
        self.load_timer(window, cx);
        let backend = self.backend.clone();
        self.poll = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let result = receive(backend.snapshot(
                    selection.project.clone(),
                    selection.session.clone(),
                    None,
                ))
                .await;
                let next = this
                    .update_in(cx, |this, window, cx| {
                        if !this.state.current(&selection) {
                            return None;
                        }
                        match result {
                            Ok(snapshot) => {
                                let was_running =
                                    this.state.snapshot.as_ref().is_some_and(|s| s.running);
                                let was_asking = this
                                    .state
                                    .snapshot
                                    .as_ref()
                                    .is_some_and(|s| !s.approvals.is_empty());
                                let running = snapshot.running;
                                // What to tell a user who is looking elsewhere.
                                let event = if was_running && !running {
                                    Some(match snapshot.error {
                                        Some(_) => crate::notify::Event::Failed,
                                        None => crate::notify::Event::Done,
                                    })
                                } else if !was_asking && !snapshot.approvals.is_empty() {
                                    Some(crate::notify::Event::Approval)
                                } else {
                                    None
                                };
                                let active_runs = panel::active_runs(&snapshot.run_cards);
                                if this.state.accept(&selection, snapshot) {
                                    if let Some(event) = event {
                                        this.tell_user(event, window);
                                    }
                                    // A turn that starts or ends may be the
                                    // timer's, which then fires next later.
                                    if running != was_running {
                                        this.load_timer(window, cx);
                                    }
                                    this.update_rows();
                                    this.ensure_batches(window, cx);
                                    this.load_inline_images(window, cx);
                                    // An open search card follows its run.
                                    this.refresh_search(window, cx);
                                    if running {
                                        this.watch_activity(window, cx);
                                        // Usage grows while the turn runs.
                                        this.status_tick = this.status_tick.wrapping_add(1);
                                        if this.status_tick % 7 == 0 {
                                            this.load_status(window, cx);
                                        }
                                        // A batch the turn delegates shows
                                        // up while the turn waits for it.
                                        if this.agents_wanted() && this.status_tick % 3 == 0 {
                                            this.load_batches(window, cx);
                                        }
                                    } else if was_running {
                                        this.load_status(window, cx);
                                        this.load_turn_tokens(window, cx);
                                        // The turn may have written files and
                                        // changed what needs attention.
                                        this.refresh_lists(window, cx);
                                        this.reload_files(window, cx);
                                        this.reload_artifacts(window, cx);
                                        this.load_reply_artifacts(window, cx);
                                        if this.agents_wanted() {
                                            this.load_batches(window, cx);
                                        }
                                        // The assistant's turn may have
                                        // changed the plan.
                                        if this
                                            .state
                                            .session
                                            .as_deref()
                                            .is_some_and(wisp_server::desktop::is_assistant)
                                        {
                                            this.load_assistant_plan(window, cx);
                                        }
                                        // What could be asked next arrives
                                        // a little after the answer.
                                        this.await_after_turn(window, cx);
                                    }
                                    this.mark_seen(window, cx);
                                    cx.notify();
                                }
                                panel::poll_delay(running, active_runs)
                            }
                            Err(error) => {
                                this.error = Some(error);
                                cx.notify();
                                None
                            }
                        }
                    })
                    .ok()
                    .flatten();
                let Some(delay) = next else {
                    break;
                };
                cx.background_executor().timer(delay).await;
            }
        }));
    }

    /// Forgets the earlier pages loaded behind the latest one; they are
    /// asked for again as the user scrolls up.
    pub(super) fn drop_history(&mut self) {
        self.history.clear();
        self.history_epoch = self.history_epoch.wrapping_add(1);
        self.loading_history = false;
    }

    fn update_rows(&mut self) {
        let Some(snapshot) = &self.state.snapshot else {
            return;
        };
        // The latest page moved under the earlier pages loaded behind it (a
        // turn ended, or a timer's firing took rows away): they would leave
        // a gap or show rows that are gone, so they go, as the web client's
        // do. Everything placed by row index is placed again below.
        let latest = (snapshot.next_before_seq, snapshot.user_offset);
        if !self.history.is_empty()
            && !crate::outline::earlier_pages_hold(self.history_under, latest)
        {
            self.drop_history();
        }
        let Some(snapshot) = &self.state.snapshot else {
            return;
        };
        if self.history.is_empty() {
            self.before = snapshot.next_before_seq;
            self.history_under = latest;
            self.earlier_users = snapshot.user_offset;
        }
        let mut rows = self.history.clone();
        rows.extend(snapshot.items.clone());
        // Keep cached heights for unchanged rows. Streaming invalidates just its
        // changing suffix; GPUI lays out only visible rows and the overdraw area.
        let prefix = self
            .rows
            .iter()
            .zip(&rows)
            .take_while(|(a, b)| present::same_row(a, b))
            .count();
        self.list
            .splice(prefix..self.rows.len(), rows.len() - prefix);
        self.rows = rows;
        self.place_generated();
        self.place_agent_cards();
        self.place_run_cards();
        // The same words, in the transcript as it is now.
        if self.layers.contains(Layer::Find) {
            self.find.refresh(&self.rows);
        }
    }

    fn older(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(before) = self.before else {
            return;
        };
        if self.loading_history {
            return;
        }
        self.loading_history = true;
        let epoch = self.history_epoch;
        let selection = self.state.selection();
        let reply = self.backend.snapshot(
            selection.project.clone(),
            selection.session.clone(),
            Some(before),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Read before the loaded pages were dropped: it no longer
                // joins what is shown.
                if !this.state.current(&selection) || this.history_epoch != epoch {
                    return;
                }
                this.loading_history = false;
                match result {
                    Ok(page) => {
                        let mut history = page.items;
                        history.append(&mut this.history);
                        this.history = history;
                        this.before = page.next_before_seq;
                        this.earlier_users = page.user_offset;
                        this.update_rows();
                        this.ensure_batches(window, cx);
                        this.load_inline_images(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Loads every earlier page of the open conversation, so its outline
    /// lists all of it.
    fn load_whole_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.before.is_none() || self.loading_history {
            return;
        }
        self.loading_history = true;
        let epoch = self.history_epoch;
        let selection = self.state.selection();
        let backend = self.backend.clone();
        let mut before = self.before;
        cx.spawn_in(window, async move |this, cx| {
            // Newest first, as they are asked for.
            let mut pages: Vec<Vec<Item>> = Vec::new();
            let mut failure = None;
            // The user's messages before the earliest page reached.
            let mut earliest_offset = None;
            while let Some(seq) = before {
                let page = receive(backend.snapshot(
                    selection.project.clone(),
                    selection.session.clone(),
                    Some(seq),
                ))
                .await;
                match page {
                    Ok(page) => {
                        // A page that moves no further back ends the walk.
                        before = crate::outline::next_page(seq, page.next_before_seq);
                        earliest_offset = Some(page.user_offset);
                        pages.push(page.items);
                    }
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) || this.history_epoch != epoch {
                    return;
                }
                this.loading_history = false;
                this.history = crate::outline::stitch(pages, std::mem::take(&mut this.history));
                this.before = before;
                if let Some(offset) = earliest_offset {
                    this.earlier_users = offset;
                }
                this.update_rows();
                this.ensure_batches(window, cx);
                this.load_inline_images(window, cx);
                if let Some(error) = failure {
                    this.error = Some(error);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Starts a conversation that drafts a problem report with the model,
    /// as the WebView's Feedback does: in the open project, or the built-in
    /// one, with what is known about the app in the composer, to send once
    /// the problem is described.
    pub(crate) fn report_problem(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        for page in [
            Layer::Settings,
            Layer::Calendar,
            Layer::Journey,
            Layer::Papers,
            Layer::Schedules,
            Layer::Mandates,
            Layer::Favorites,
        ] {
            self.layers.close(page);
        }
        let project = match (&self.state.project, self.screen) {
            (Some(project), Screen::Project) => project.clone(),
            _ => {
                let project = wisp_server::desktop::BUILT_IN_PROJECT.to_owned();
                self.enter_project(project.clone(), window, cx);
                project
            }
        };
        let options = self.options();
        let model = crate::settings::model_label(self.settings.current(), &options.model);
        let prompt = crate::links::issue_report_prompt(
            wisp_server::desktop::VERSION,
            std::env::consts::OS,
            std::env::consts::ARCH,
            &model,
        );
        self.pending = true;
        let selection = self.state.selection();
        let reply = self.backend.create_conversation(project);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(id) => {
                        this.select_session(id, window, cx);
                        this.load_sessions(window, cx);
                        this.state.save_draft(prompt.clone());
                        this.fill_composer(prompt, window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn create_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.state.project.is_none() {
            return;
        }
        self.pending = true;
        let selection = self.state.selection();
        // Through the desktop's own call, so the conversation starts the
        // way Settings says new ones should.
        let Some(project) = selection.project.clone() else {
            return;
        };
        let reply = self.backend.create_conversation(project);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(id) => {
                        this.select_session(id, window, cx);
                        this.load_sessions(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Nothing is written into a conversation that cannot take it: not a
        // message, not a command, not one queued for later.
        if self.read_only().is_some() {
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        // A message that is only a command is run, not sent; in an ACP
        // agent's conversation every message is the agent's to read, its
        // own commands included.
        if self.options().acp_agent.is_empty() && self.run_typed_slash(&text, window, cx) {
            return;
        }
        // While a turn runs the message waits for it to end.
        if self.state.snapshot.as_ref().is_some_and(|s| s.running) {
            return self.enqueue(text, window, cx);
        }
        self.send_text(text, true, window, cx);
    }

    /// Sends `text` as the next message. An answer picked on a question card
    /// is sent as it is and leaves whatever is being typed alone.
    fn send_text(
        &mut self,
        text: String,
        from_composer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // What the composer has attached goes with what it has typed.
        let carried = match (&self.state.session, from_composer) {
            (Some(session), true) => self.chips.get(session).cloned().unwrap_or_default(),
            _ => Pending::default(),
        };
        self.deliver(text, carried, from_composer, window, cx);
    }

    /// Hands a message to the host.
    fn deliver(
        &mut self,
        text: String,
        carried: Pending,
        from_composer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.read_only().is_some() {
            return;
        }
        let attachments = carried.attachments();
        let referenced = carried.reference_chips();
        // Files alone are a message; the host names it after the first one.
        let sendable = if text.trim().is_empty() && !attachments.is_empty() {
            "files"
        } else {
            text.as_str()
        };
        if !self.state.can_send(sendable, self.pending) || self.loading_history {
            return;
        }
        self.pending = true;
        self.error = None;
        self.poll = None;
        let reply = self.backend.send_message(
            project,
            SendRequest {
                session_id: session.clone(),
                request_id: uuid::Uuid::new_v4().to_string(),
                message: text.trim().to_owned(),
                resume: false,
                resume_after: None,
                attachments,
                references: carried.references(),
                acp_agent_id: None,
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                // The turn runs whether or not its conversation is still open.
                if result.is_ok() {
                    this.watch_activity(window, cx);
                }
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(_) => {
                        // What it carried, shown on it while this window
                        // is open.
                        if !referenced.is_empty() {
                            let sent = this.sent_references.entry(session.clone()).or_default();
                            sent.push((text.trim().to_owned(), referenced));
                            let over = sent.len().saturating_sub(crate::compose::SENT_KEPT);
                            sent.drain(..over);
                        }
                        // Clear only after durable acceptance. Failure preserves the draft.
                        if from_composer {
                            this.composer
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            this.state.save_draft(String::new());
                            this.chips.remove(&session);
                        }
                        this.drop_history();
                        if let Some(snapshot) = &mut this.state.snapshot {
                            snapshot.running = true;
                        }
                        this.load_sessions(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                this.start_poll(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Why the open conversation cannot be written to, when it cannot.
    pub(super) fn read_only(&self) -> Option<compose::ReadOnly> {
        let snapshot = self.state.snapshot.as_ref()?;
        let session = self.state.session.as_ref();
        let parent = session.and_then(|session| self.dispatched.get(session));
        let source = session.and_then(|session| self.branched.get(session));
        compose::read_only(
            snapshot,
            parent.map(String::as_str),
            source.map(String::as_str),
        )
    }

    /// What the open conversation's messages and menu offer that would
    /// change it or start from it, as the host allows it.
    pub(super) fn offered(&self) -> compose::Offered {
        compose::offered(self.read_only().as_ref(), self.open_is_branch())
    }

    /// What is attached to the message being written in this conversation.
    fn attached(&self) -> &[Chip] {
        self.state
            .session
            .as_ref()
            .and_then(|session| self.chips.get(session))
            .map_or(&[], Pending::chips)
    }

    fn attach(&mut self, chip: Chip, cx: &mut Context<Self>) {
        if let Some(session) = self.state.session.clone() {
            self.chips.entry(session).or_default().add(chip);
            cx.notify();
        }
    }

    fn detach(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(pending) = self
            .state
            .session
            .as_ref()
            .and_then(|session| self.chips.get_mut(session))
        {
            pending.remove(index);
            cx.notify();
        }
    }

    /// Picks files with the system dialog. Files outside the project are
    /// copied into its `uploads` folder; files inside it are used in place.
    fn attach_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only().is_some() {
            return;
        }
        let selection = self.state.selection();
        let (Some(project), Some(_)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(tr("Attach").into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let picked = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                match picked {
                    Some(files) if !files.is_empty() => {
                        receive(backend.import_attachments(project, files)).await
                    }
                    _ => Ok(vec![]),
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.attach_imported(&selection, result, window, cx)
            });
        })
        .detach();
    }

    /// Attaches files dropped onto the conversation, the way the picker
    /// does: outside files are copied into `uploads`, inside ones are used
    /// in place. Folders are left out, and it says so.
    pub(super) fn attach_dropped(
        &mut self,
        paths: &[std::path::PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only().is_some() {
            return;
        }
        let selection = self.state.selection();
        let (Some(project), Some(_)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let (files, left_out) = compose::dropped_files(paths, |path| path.is_file());
        self.error = compose::drop_note(left_out);
        if files.is_empty() {
            return cx.notify();
        }
        let reply = self.backend.import_attachments(project, files);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.attach_imported(&selection, result, window, cx)
            });
        })
        .detach();
        cx.notify();
    }

    /// A picture on the clipboard, pasted while the composer has focus, is
    /// kept in the project's `uploads` and put on the message being
    /// written. Text on the clipboard is the composer's to paste, so a
    /// picture that comes with text is left to it. Whether it took the
    /// paste.
    pub(super) fn paste_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.read_only().is_some() || !self.composer.focus_handle(cx).is_focused(window) {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let has_text = item.entries().iter().any(
            |entry| matches!(entry, ClipboardEntry::String(text) if !text.text().trim().is_empty()),
        );
        let Some(image) = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image),
            _ => None,
        }) else {
            return false;
        };
        if has_text {
            return false;
        }
        let selection = self.state.selection();
        let (Some(project), Some(_)) = (selection.project.clone(), selection.session.clone())
        else {
            return false;
        };
        let extension = match image.format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Gif => "gif",
            ImageFormat::Svg => "svg",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
        };
        let name = compose::pasted_name(extension, chrono::Local::now().naive_local());
        let reply = self
            .backend
            .import_pasted(project, name, image.bytes.clone());
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await.map(|path| vec![path]);
            let _ = this.update_in(cx, |this, window, cx| {
                this.attach_imported(&selection, result, window, cx)
            });
        })
        .detach();
        true
    }

    /// Puts imported files on the message being written.
    fn attach_imported(
        &mut self,
        selection: &crate::state::Selection,
        result: Result<Vec<String>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The files belong to the conversation they were picked for.
        if !self.state.current(selection) {
            return;
        }
        match result {
            Ok(paths) => {
                for path in paths {
                    self.attach(Chip::File(path), cx);
                }
                self.reload_files(window, cx);
                self.focus_composer(window, cx);
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn load_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = self.backend.conversation_status(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(status) => this.status = Some(status),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads what each saved turn of the open conversation used. Nothing is
    /// said when it cannot be read: the line under an answer is left out.
    fn load_turn_tokens(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = self.backend.turn_tokens(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                this.turn_tokens = result.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    /// Saves the conversation's options; they apply from its next turn.
    fn set_options(
        &mut self,
        options: ConversationOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        self.error = None;
        let reply = self
            .backend
            .set_conversation_options(project, session, options);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match (result, &mut this.status) {
                    (Ok(options), Some(status)) => status.options = options,
                    (Ok(_), None) => {}
                    (Err(error), _) => this.error = Some(error),
                }
                // A host chosen for new work is attached with it.
                this.reload_artifacts(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn options(&self) -> ConversationOptions {
        self.status
            .as_ref()
            .map(|status| status.options.clone())
            .unwrap_or_default()
    }

    /// Turning full access on asks first; turning it off does not.
    fn toggle_full_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for menu in Layer::COMPOSER_MENUS {
            self.layers.close(menu);
        }
        if self.options().full_access {
            self.layers.close(Layer::ConfirmFullAccess);
            let options = ConversationOptions {
                full_access: false,
                ..self.options()
            };
            self.set_options(options, window, cx);
        } else if !self.layers.close(Layer::ConfirmFullAccess) {
            self.layers.open(Layer::ConfirmFullAccess);
        }
        cx.notify();
    }

    fn confirm_full_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.close(Layer::ConfirmFullAccess) {
            return;
        }
        let options = ConversationOptions {
            full_access: true,
            ..self.options()
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Opens one of the menus above the composer, or closes it if it is the
    /// one already open. Only one shows at a time.
    fn toggle_composer_menu(&mut self, menu: Layer, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ConfirmFullAccess);
        let was_open = self.layers.contains(menu);
        for other in Layer::COMPOSER_MENUS {
            self.layers.close(other);
        }
        if was_open {
            self.focus_composer(window, cx);
        } else {
            self.layers.open(menu);
        }
        cx.notify();
    }

    /// An empty id follows the default model.
    fn choose_model(&mut self, model: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ModelMenu);
        let options = ConversationOptions {
            model,
            ..self.options()
        };
        self.set_options(options, window, cx);
        // The header names the model this conversation runs on.
        self.start_poll(window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// The model this conversation's sub-agents work on: `None` as in
    /// Settings, an empty id this conversation's own model.
    fn choose_subagent_model(
        &mut self,
        choice: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::ModelMenu);
        let options = ConversationOptions {
            subagent_model: choice,
            ..self.options()
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Puts a quick action's request into the composer, after whatever is
    /// being written, with the caret at its end.
    fn use_quick_action(&mut self, prompt: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::QuickMenu);
        let text = crate::compose::with_quick_action(&self.composer.read(cx).value(), &prompt);
        self.state.save_draft(text.clone());
        self.fill_composer(text, window, cx);
        cx.notify();
    }

    /// Starts a quick action's workflow as agents on what is typed, in a
    /// new conversation shown with its Agents tab, where the batch is
    /// approved or discarded. What was typed went to it, so the composer
    /// is emptied.
    fn start_quick_action(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::QuickMenu);
        let Some(project) = self.state.project.clone() else {
            return;
        };
        let typed = self.composer.read(cx).value().to_string();
        self.error = None;
        let reply = self.backend.start_quick_action(project.clone(), id, typed);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(drafted) => {
                        this.state.save_draft(String::new());
                        this.fill_composer(String::new(), window, cx);
                        this.open_session(project, drafted.session, window, cx);
                        this.show_panel_tab(PanelTab::Agents, window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Sets the composer's text with the caret at its end, where writing
    /// goes on, and gives it the keyboard. Every fill goes through here, so
    /// none leaves the caret at the start of what it put in.
    pub(crate) fn fill_composer(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.update(cx, |input, cx| {
            let (line, character) = crate::compose::end_of(&text);
            input.set_value(text, window, cx);
            input.set_cursor_position(
                gpui_component::input::Position { line, character },
                window,
                cx,
            );
            input.focus(window, cx);
        });
    }

    /// Starts this conversation as `expert`, or as none with an empty id.
    fn choose_expert(&mut self, expert: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::ExpertMenu);
        let options = ConversationOptions {
            expert,
            ..self.options()
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Opens the list of one of the agent's modes or options, or closes it.
    fn toggle_acp_setting(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let same = self.acp_setting_open.as_deref() == Some(id.as_str())
            && self.layers.contains(Layer::AcpSettingMenu);
        self.toggle_composer_menu(Layer::AcpSettingMenu, window, cx);
        self.acp_setting_open = (!same).then_some(id);
        cx.notify();
    }

    /// Sets the agent's mode (`setting` is "mode") or one of its options,
    /// and shows what it reports back.
    fn set_acp_setting(
        &mut self,
        setting: String,
        pick: wisp_server::desktop::AcpPick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers.close(Layer::AcpSettingMenu);
        self.acp_setting_open = None;
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = match (setting.as_str(), pick) {
            ("mode", wisp_server::desktop::AcpPick::Choice(mode)) => {
                self.backend.set_acp_mode(project, session, mode)
            }
            (_, pick) => self.backend.set_acp_option(project, session, setting, pick),
        };
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(controls) => {
                        if let Some(status) = this.status.as_mut() {
                            status.acp = Some(controls);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Gives this conversation, before its first message, to an ACP agent.
    /// The choice is for good; Wisp needs a new conversation.
    fn choose_acp_agent(&mut self, agent: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::AgentMenu);
        if agent != self.options().acp_agent {
            let options = ConversationOptions {
                acp_agent: agent,
                ..self.options()
            };
            self.set_options(options, window, cx);
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Has each turn of this conversation that is worth it reviewed when
    /// it ends, or stops doing so.
    fn toggle_auto_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = self.options();
        let options = ConversationOptions {
            auto_review: !options.auto_review,
            ..options
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Plan first: the model investigates and proposes; nothing is changed.
    fn toggle_plan_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = self.options();
        let options = ConversationOptions {
            plan_first: !options.plan_first,
            ..options
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Leaves plan mode and, when `execute`, tells the model to go ahead:
    /// with what is typed in the composer, or the standard words. Leaving
    /// first matters: a message sent while still planning would be refused
    /// the tools it needs.
    fn decide_plan(&mut self, execute: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.pending {
            return;
        }
        self.pending = true;
        self.error = None;
        let options = ConversationOptions {
            plan_first: false,
            ..self.options()
        };
        let reply = self
            .backend
            .set_conversation_options(project, session, options);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(options) => {
                        if let Some(status) = &mut this.status {
                            status.options = options;
                        }
                        if execute {
                            let typed = this.composer.read(cx).value().to_string();
                            if typed.trim().is_empty() {
                                this.send_text(controls::APPROVE_PLAN.into(), false, window, cx);
                            } else {
                                this.send(window, cx);
                            }
                        } else {
                            this.start_poll(window, cx);
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Where this conversation's work runs: empty follows Settings.
    fn choose_runs_on(&mut self, runs_on: String, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::RunsOnMenu);
        let options = ConversationOptions {
            runs_on,
            ..self.options()
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Fast mode on or off, from the next message.
    fn toggle_fast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for menu in Layer::COMPOSER_MENUS {
            self.layers.close(menu);
        }
        let options = ConversationOptions {
            fast: !self.options().fast,
            ..self.options()
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn choose_effort(&mut self, effort: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.layers.close(Layer::EffortMenu);
        let options = ConversationOptions {
            reasoning_effort: effort.into(),
            ..self.options()
        };
        self.set_options(options, window, cx);
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn answer_approval(
        &mut self,
        approval: String,
        approved: bool,
        remember: wisp_server::desktop::Remember,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        if self.pending {
            return;
        }
        self.pending = true;
        self.error = None;
        let reply = self
            .backend
            .answer_approval(project, session, approval, approved, remember);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                // An answered approval resumes a turn the watcher may have
                // stopped following while it waited.
                this.watch_activity(window, cx);
                if !this.state.current(&selection) {
                    return;
                }
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.start_poll(window, cx);
                this.load_status(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Sends a proposed plan back with what should change, which the
    /// model reads with the refusal.
    fn send_plan_feedback(
        &mut self,
        approval: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let feedback = self.plan_feedback.read(cx).value().trim().to_owned();
        if self.pending || feedback.is_empty() {
            return;
        }
        self.pending = true;
        self.error = None;
        let reply = self
            .backend
            .answer_with_feedback(project, session, approval, feedback);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                this.watch_activity(window, cx);
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(()) => this
                        .plan_feedback
                        .update(cx, |input, cx| input.set_value("", window, cx)),
                    Err(error) => this.error = Some(error),
                }
                this.start_poll(window, cx);
                this.load_status(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Stops one of the conversation's interpreters, or dismisses a
    /// stopped one, and reads the status again.
    fn stop_runtime(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = self.backend.stop_runtime(project, session, id);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.load_status(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn revoke_allowed_tools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let (Some(project), Some(session)) = (selection.project.clone(), selection.session.clone())
        else {
            return;
        };
        let reply = self.backend.revoke_allowed_tools(project, session);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.load_status(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Copies a transcript row's text and shows a tick on it for a moment.
    /// What a row says, without the files a user message carried.
    fn row_text(&self, index: usize) -> Option<String> {
        let item = self.rows.get(index)?;
        Some(if item.role == "user" {
            compose::user_text(item)
        } else {
            item.text.clone()
        })
    }

    /// Puts a message in the composer as a quote, after what is being
    /// written, with the caret ready below it.
    fn quote_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.row_text(index) else {
            return;
        };
        let filled = compose::with_quote(&self.composer.read(cx).value(), &text);
        self.state.save_draft(filled.clone());
        self.fill_composer(filled, window, cx);
        cx.notify();
    }

    /// Opens the aside with a message quoted in its question field.
    fn ask_aside_about_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.row_text(index) else {
            return;
        };
        // The aside's field is one line: a line break there would not draw.
        let filled = compose::with_inline_quote(&self.aside_input.read(cx).value(), &text);
        self.open_aside(window, cx);
        self.aside_input.update(cx, |input, cx| {
            let (line, character) = compose::end_of(&filled);
            input.set_value(filled, window, cx);
            input.set_cursor_position(
                gpui_component::input::Position { line, character },
                window,
                cx,
            );
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn copy_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.rows.get(index) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(item.text.clone()));
        self.copied = Some(index);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.copied == Some(index) {
                    this.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn control(
        &mut self,
        command: &'static str,
        mut args: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        self.pending = true;
        self.error = None;
        let selection = self.state.selection();
        args["session_id"] = json!(selection.session);
        let reply = self
            .backend
            .request::<Value>(command, selection.project.clone(), args);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                // An answered approval resumes a turn the watcher may have
                // stopped following while it waited.
                this.watch_activity(window, cx);
                if !this.state.current(&selection) {
                    return;
                }
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.start_poll(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.ready {
            return;
        }
        self.pending = true;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Open research folder").into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let paths = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                match paths.and_then(|mut paths| paths.pop()) {
                    Some(path) => receive(backend.register_project(path)).await.map(Some),
                    None => Ok(None),
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(Some(id)) => {
                        this.enter_project(id, window, cx);
                        this.load_projects(window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .relative()
            .size_full()
            // Body text: what has no size of its own is this, not a rem.
            .text_sm()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);
        if !self.ready {
            let bar = surfaces::window_bar(None, window, cx);
            return root.child(surfaces::window_frame(bar, self.render_startup(cx)));
        }
        // A page takes the whole window under its bar; of two, the one
        // opened last shows. The home page's top bar and a project's tabs
        // and actions are in the window's bar; the other pages have a plain
        // one.
        let page = self.layers.top_of(&[
            Layer::Settings,
            Layer::Favorites,
            Layer::Journey,
            Layer::Papers,
            Layer::Schedules,
            Layer::Mandates,
            Layer::Calendar,
        ]);
        let mut bar_content = None;
        let content: AnyElement = if page == Some(Layer::Settings) {
            self.render_settings(cx).into_any_element()
        } else if page == Some(Layer::Calendar) {
            self.render_calendar(cx).into_any_element()
        } else if page == Some(Layer::Journey) {
            self.render_journey(cx).into_any_element()
        } else if page == Some(Layer::Papers) {
            self.render_papers(cx).into_any_element()
        } else if page == Some(Layer::Schedules) {
            self.render_schedules(cx).into_any_element()
        } else if page == Some(Layer::Mandates) {
            self.render_mandates(cx).into_any_element()
        } else if page == Some(Layer::Favorites) {
            self.render_favorites(window, cx).into_any_element()
        } else {
            match self.screen {
                Screen::Home => {
                    bar_content = Some(self.render_home_bar(window, cx).into_any_element());
                    self.render_home(cx).into_any_element()
                }
                Screen::Project => {
                    bar_content = Some(self.render_project_bar(window, cx).into_any_element());
                    let mut row = div().h_flex().size_full();
                    if !self.sidebar_hidden {
                        row = row.child(self.render_sidebar(window, cx));
                    }
                    row = row.child(self.render_conversation(window, cx));
                    if self.layers.contains(Layer::FilesPanel) {
                        row = row.child(self.render_files(window, cx));
                    }
                    row.into_any_element()
                }
            }
        };
        let bar = surfaces::window_bar(bar_content, window, cx);
        let root = root.child(surfaces::window_frame(bar, content));
        // Conversations of other projects that finished, in a corner over
        // the page and under the dialogs.
        let root = root.children(self.render_session_notices(cx));
        // Last, so they are drawn over whichever screen or layer is showing.
        let root = if self.layers.contains(Layer::ProjectSettings) {
            root.child(self.render_project_settings(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::JoinSyncedProject) {
            root.child(self.render_join_synced(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::NewProject) {
            root.child(self.render_new_project(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::Examples) {
            root.child(self.render_examples(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::StartExploration) {
            root.child(self.render_start_exploration(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::ExplorationPreview) {
            root.child(self.render_exploration_preview(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::Capabilities) {
            root.child(self.render_capabilities(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::PrivacyMode) {
            root.child(self.render_privacy(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::RunReview) {
            root.child(self.render_run_review(cx))
        } else {
            root
        };
        let root = match &self.journey_run {
            Some(open) if self.layers.contains(Layer::JourneyRun) => {
                root.child(self.render_journey_run(open, cx))
            }
            _ => root,
        };
        let root = if self.layers.contains(Layer::Trajectory) {
            root.child(self.render_trajectory(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::Review) {
            root.child(self.render_review(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::ModelView) {
            root.child(self.render_model_view(cx))
        } else {
            root
        };
        // Over What the model is sent when it was opened from there.
        let root = if self.layers.contains(Layer::CompactContext) {
            root.child(self.render_compact_dialog(cx))
        } else {
            root
        };
        // Over What the model is sent too, when it was opened from there.
        let root = if self.layers.contains(Layer::Handoff) {
            root.child(self.render_handoff(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::SessionReuse) {
            root.child(self.render_session_reuse(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::ResearchArchive) {
            root.child(self.render_research_archive(window, cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::ConfirmDiscardArchive) {
            root.child(self.render_discard_archive(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::ShareMessages) {
            root.child(self.render_share(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::BranchMerge) {
            root.child(self.render_branch_merge(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::WebViewImport) {
            root.child(self.render_webview_import(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::ImportConversations) {
            root.child(self.render_imports(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::Onboarding) {
            root.child(self.render_onboarding(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::ConfirmClearMemory) {
            root.child(self.render_clear_memory(cx))
        } else {
            root
        };
        let root = match self.render_remove_staged(cx) {
            Some(dialog) => root.child(dialog),
            None => root,
        };
        let root = if self.layers.contains(Layer::FileChangedOnDisk) {
            root.child(self.render_file_changed(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::ConfirmCloseUnsaved) {
            root.child(self.render_close_unsaved(cx))
        } else {
            root
        };
        // What the browser asks: the question about a human over the one
        // about tabs, as the WebView draws them.
        let root = if self.layers.contains(Layer::BrowserTabCleanup) {
            root.children(self.render_browser_cleanup(cx))
        } else {
            root
        };
        let root = if self.layers.contains(Layer::BrowserNeedsHuman) {
            root.children(self.render_browser_needs_human(cx))
        } else {
            root
        };
        if self.layers.contains(Layer::Palette) {
            root.child(self.render_palette(cx))
        } else {
            root
        }
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

    #[test]
    fn the_composer_keys_apply_only_in_a_composer_where_return_sends() {
        let bindings = super::composer_bindings();
        assert_eq!(bindings.len(), 2);
        let sends = stack(&["Workspace", crate::settings::SEND_ON_ENTER, "Input"]);
        let modifier = stack(&["Workspace", "Composer", "Input"]);
        let elsewhere = stack(&["Workspace", "Input"]);
        for binding in &bindings {
            let applies = binding.predicate().unwrap();
            assert!(applies.eval_inner(&sends, &sends));
            assert!(!applies.eval_inner(&modifier, &modifier));
            assert!(!applies.eval_inner(&elsewhere, &elsewhere));
        }
    }
}
