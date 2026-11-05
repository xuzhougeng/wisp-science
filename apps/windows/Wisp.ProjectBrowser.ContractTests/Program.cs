using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

// Stand in for an incompatible desktop intercepting the settings-host launch.
if (args.SequenceEqual(new[] { "--native-settings-host" }))
{
    if (int.TryParse(Environment.GetEnvironmentVariable("WISP_TEST_HOST_EXIT_CODE"), out var exitCode)) Environment.ExitCode = exitCode;
    return;
}

if (args.Length is 2 or 3 && args[0] == "--database")
{
    await BrowserTests.FakeServiceAsync(args[1]);
    return;
}

if (args.Length != 1)
    throw new ArgumentException("Pass contracts/project-browser/v1/projects.json");

var response = JsonSerializer.Deserialize<ProjectBrowserResponse>(File.ReadAllText(args[0]))
    ?? throw new InvalidOperationException("Missing response");
if (response.Schema != ProjectBrowserProtocol.Schema || response.Id != "projects-1"
    || response.Type != "projects" || response.ActivitySource != ProjectBrowserProtocol.PersistedOnly)
    throw new InvalidOperationException("Protocol envelope drift");
var project = response.Projects?.Single() ?? throw new InvalidOperationException("Missing project");
if (project.Id != "research-1" || project.Name != "RNA-seq 研究"
    || project.WorkspaceDirectory != "/Users/researcher/Projects/RNA seq"
    || project.SessionCount != 3 || project.ArtifactCount != 2
    || project.NeedsYouCount != 1 || !project.Starred || !project.SyncConfigured
    || project.LastSyncedAt != 1789500000)
    throw new InvalidOperationException("Project DTO drift");
Console.WriteLine("Shared Rust / Swift / C# project-browser fixture passed.");
var results = new ContractSuiteResults();
await results.RunAsync(() => HomeProjectSyncTests.RunAsync());
results.Run(() => NativeAcpChecksTests.Run());

var fixtureDirectory = Path.GetDirectoryName(args[0])!;
await results.RunAsync(() => HomeActivityContractTests.RunAsync(Path.Combine(fixtureDirectory, "../../native-projects/v1/home-activity.json")));
var sessions = JsonSerializer.Deserialize<ProjectBrowserResponse>(File.ReadAllText(Path.Combine(fixtureDirectory, "sessions.json")))!;
if (sessions.Type != "sessions" || sessions.Sessions?.Single().ProjectId != "research-1"
    || sessions.Sessions.Single().Status != "needs_you")
    throw new InvalidOperationException("Session DTO drift");
var transcript = JsonSerializer.Deserialize<ProjectBrowserResponse>(File.ReadAllText(Path.Combine(fixtureDirectory, "transcript.json")))!;
if (transcript.Type != "transcript" || transcript.Messages?.Count != 2
    || transcript.Messages[0].Sequence != 6 || transcript.NextBeforeSeq != 6)
    throw new InvalidOperationException("Transcript DTO drift");

await results.RunAsync(() => BrowserTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => ProjectChatHistoryTests.RunAsync(Path.Combine(fixtureDirectory, "chat-matches.json")));
var star = JsonSerializer.Deserialize<SetProjectStarredRequest>(File.ReadAllText(Path.Combine(fixtureDirectory, "set-project-starred.json")))!;
if (star.Schema != ProjectBrowserProtocol.Schema || star.Id != "projects-1"
    || star.Type != "set_project_starred" || star.ProjectId != "research-1" || !star.Starred)
    throw new InvalidOperationException("Project star command drift");
var encodedStar = JsonSerializer.SerializeToElement(star);
if (!encodedStar.GetProperty("starred").GetBoolean() || encodedStar.GetProperty("project_id").GetString() != "research-1")
    throw new InvalidOperationException("Project star serialization drift");

await results.RunAsync(() => NativeSettingsContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-settings/v1"))));
await results.RunAsync(() => NativeShareExportTests.RunAsync());
await results.RunAsync(() => NativeFilesTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeFileSelectionTests.RunAsync());
await results.RunAsync(() => NativeProjectFilesTests.RunAsync());
await results.RunAsync(() => NativeAttachmentTests.RunAsync());
await results.RunAsync(() => NativeClipboardTests.RunAsync());
await results.RunAsync(() => NativeSidebarReferenceTests.RunAsync());
await results.RunAsync(() => NativeAcpConversationTests.RunAsync());
await results.RunAsync(() => NativeAcpSelectionTests.RunAsync());
await results.RunAsync(() => NativeDraftModelSelectionTests.RunAsync());
await results.RunAsync(() => NativeModelCompactionTests.RunAsync());
await results.RunAsync(() => NativeCompactionHistoryTests.RunAsync());
results.Run(() => NativeAcpQuestionTests.Run());
results.Run(() => NativeComposerMenuTests.Run());
results.Run(() => NativeComposerModelMenuTests.Run());
results.Run(() => NativeComputeMenuTests.Run());
await results.RunAsync(() => NativeConversationHistoryTests.RunAsync());
await results.RunAsync(() => NativeConversationExcerptTests.RunAsync());
await results.RunAsync(() => NativeTranscriptSelectionTests.RunAsync());
results.Run(() => NativeSourceSelectionTests.Run());
results.Run(() => NativePreviewSelectionTests.Run());
await results.RunAsync(() => NativeReviewPresentationTests.RunAsync());
await results.RunAsync(() => NativeConversationQueueTests.RunAsync());
await results.RunAsync(() => NativeExecutionReadingTests.RunAsync());
await results.RunAsync(() => NativePlanProposalTests.RunAsync());
await results.RunAsync(() => NativeHostReconnectTests.RunAsync());
await results.RunAsync(() => NativeSearchTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-search/v1/search.json"))));
results.Run(() => NativePanelPresentationTests.Run(args[0]));
await results.RunAsync(() => NativeSessionImportTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-session-import/v1/archive.json"))));
await results.RunAsync(() => NativeExternalImportTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-session-import/v1/external.json"))));
await results.RunAsync(() => NativeProjectContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-projects/v1/create.json"))));
await results.RunAsync(() => NativeLibraryContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-library/v1/search.json"))));
await results.RunAsync(() => NativeCalendarContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-calendar/v1/month.json"))));
await results.RunAsync(() => NativeCalendarPresentationTests.RunAsync());
await results.RunAsync(() => NativeLibraryPresentationTests.Run());
await results.RunAsync(() => NativePrivacyContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-privacy/v1/mode.json"))));
await results.RunAsync(() => NativeJourneyContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-journey/v1/range.json"))));
await results.RunAsync(() => NativePublicationContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-publication/v1/workspace.json"))));
await results.RunAsync(() => NativeConversationContractTests.Run(args[0]));

results.Run(() => NativePanelTabsTests.Run());
results.Run(() => NativeSessionRunGroupsTests.Run());
await results.RunAsync(() => NativeArtifactModalTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-conversations/v1/artifact-provenance.json"))));
await results.RunAsync(() => NativeFileProvenanceTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-conversations/v1/file-provenance.json"))));
results.Run(() => NativeCodePreviewTests.Run());
results.Run(() => NativeArtifactLayoutTests.Run());
results.Run(() => NativeWorkspaceLayoutTests.Run());
results.Run(() => NativeHomeLayoutTests.Run());
results.Run(() => NativeProjectDialogLayoutTests.Run());
results.Run(() => NativePanelViewStateTests.Run());
results.Run(() => NativeFileListPresentationTests.Run());
results.Run(() => NativeActionLayoutTests.Run());
await results.RunAsync(() => NativeSideChatPresentationTests.RunAsync());
results.Run(() => NativeToolPresentationTests.Run());
results.Run(() => NativeSettingsSearchTests.Run());
results.Run(() => NativeSettingsLayoutTests.Run());
results.Run(() => NativePreferenceIntegerTests.Run());
await results.RunAsync(() => NativeStoragePresentationTests.RunAsync());
await results.RunAsync(() => NativeUsageTests.RunAsync());
await results.RunAsync(() => NativeProjectSettingsTests.RunAsync());
await results.RunAsync(() => NativeProjectImportTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-projects/v1/recovery.json"))));
await results.RunAsync(() => NativeProjectExportTests.RunAsync());
await results.RunAsync(() => NativeProjectDeleteTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeWindowMenuTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeUpdateDialogTests.RunAsync());
results.Run(() => NativeLocalizationTests.Run());
await results.RunAsync(() => NativeInlineSettingsTests.RunAsync());
await results.RunAsync(() => NativeLocalEnvironmentTests.RunAsync());
await results.RunAsync(() => NativeQuickSetupTests.RunAsync());
await results.RunAsync(() => NativeProjectInventoryTests.RunAsync());
await results.RunAsync(() => NativeNetworkSettingsTests.RunAsync());
await results.RunAsync(() => NativeAssistantTests.RunAsync());
await results.RunAsync(() => NativeComposeMenuTests.RunAsync());
await results.RunAsync(() => NativeAssistantRemoteTests.RunAsync());
await results.RunAsync(() => NativeAutomationTests.RunAsync());
await results.RunAsync(() => NativeMandateTests.RunAsync(args[0]));
await results.RunAsync(() => NativePetPresentationTests.RunAsync());
await results.RunAsync(() => NativeWorkflowStudioTests.RunAsync());
await results.RunAsync(() => NativeWorkflowOptionsTests.RunAsync());
await results.RunAsync(() => NativeWorkflowConversionTests.RunAsync());
results.Run(() => NativeComposerCommandsTests.Run());
await results.RunAsync(() => NativeComposerReferencesTests.RunAsync(args[0]));
results.Run(() => NativeTranscriptRowsTests.Run());
await results.RunAsync(() => NativeErrorResumeTests.RunAsync());
results.Run(() => NativeTranscriptActivityTests.Run());
results.Run(() => NativeTranscriptTablesTests.Run());
results.Run(() => NativeCsvTableTests.Run());
await results.RunAsync(() => NativeArtifactActionTests.RunAsync());
await results.RunAsync(() => NativeArtifactQuoteTests.RunAsync());
await results.RunAsync(() => NativeGeneratedQuoteTests.RunAsync());
await results.RunAsync(() => NativeArtifactLibraryTests.RunAsync());
await results.RunAsync(() => NativeTerminalStreamingTests.RunAsync());
await results.RunAsync(() => WorkspaceJourneyTests.RunAsync());
await results.RunAsync(() => NativeJourneyPresentationTests.RunAsync());
results.Run(() => NativeBrowserPresentationTests.Run());
await results.RunAsync(() => NativeComposerEffortTests.RunAsync());
await results.RunAsync(() => NativeRuntimeStripTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-runtimes/v1/snapshot.json"))));
await results.RunAsync(() => NativeRuntimeInterpreterTests.RunAsync());
await results.RunAsync(() => NativeRuntimeInspectorTests.RunAsync());
await results.RunAsync(() => NativeComposerOptionsTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-conversations/v1/composer-options.json"))));
results.Run(() => NativeTranscriptTests.Run());
await results.RunAsync(() => NativeWorkspaceActionsTests.RunAsync());
await results.RunAsync(() => AppearanceSettingsTests.RunAsync());
results.Run(() => NativeMessagePresentationTests.Run());
await results.RunAsync(() => NativeGeneratedArtifactsTests.RunAsync());
await results.RunAsync(() => NativeArtifactCatalogTests.RunAsync());
await results.RunAsync(() => NativeArtifactFilesTests.RunAsync());
await results.RunAsync(() => NativeLiveAppearanceTests.RunAsync());
await results.RunAsync(() => NativeCustomThemeTests.RunAsync());
await results.RunAsync(() => WorkspaceActionTests.RunAsync());
await results.RunAsync(() => NativeShelvedSessionsTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeSessionDeletionTests.RunAsync());
await results.RunAsync(() => NativeSessionMenusTests.RunAsync());
results.Run(() => NativeSessionTreeTests.Run(fixtureDirectory));
await results.RunAsync(() => NativeSessionGroupingTests.RunAsync());
await results.RunAsync(() => NativeFolderActionsTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeBranchMergeTests.RunAsync(fixtureDirectory));
results.Run(() => NativeBranchResultsTests.Run(fixtureDirectory));
await results.RunAsync(() => NativeSessionRulesTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeExplorationTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeExplorationPreviewTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeExplorationStartTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeSessionTransferTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeDebugRequestExportTests.RunAsync(fixtureDirectory));
await results.RunAsync(() => NativeRunNavigationTests.RunAsync());
await results.RunAsync(() => NativeMethodReviewTests.RunAsync(Path.Combine(fixtureDirectory, "../../native-runtimes/v1/method-search.json")));
await results.RunAsync(() => NativeRunReviewTests.RunAsync(args[0]));
await results.RunAsync(() => WorkspaceConversationTests.RunAsync(args[0]));
await results.RunAsync(() => NativeParityTests.RunAsync());
await results.RunAsync(() => NativeContextDetailsTests.RunAsync());
results.Run(() => NativeContextHealthTests.Run());
await results.RunAsync(() => NativeHandoffTests.RunAsync());
await results.RunAsync(() => NativeIdleCompactionTests.RunAsync());
await results.RunAsync(() => NativeContextRecoveryTests.RunAsync());
results.Run(() => NativeContextPanelLayoutTests.Run());
await results.RunAsync(() => NativeModelContextTests.RunAsync());
await results.RunAsync(() => NativeSettingsEditorTests.RunAsync());
await results.RunAsync(() => NativeSubagentModelTests.RunAsync());
await results.RunAsync(() => NativePermissionTests.RunAsync());
await results.RunAsync(() => NativeCredentialTests.RunAsync());
await results.RunAsync(() => NativeEnvironmentTests.RunAsync());
await results.RunAsync(() => NativeReviewerTests.RunAsync());
await results.RunAsync(() => NativeSpecialistTransferTests.RunAsync());
results.Run(() => NativeWindowEscapeTests.Run());
await results.RunAsync(() => NativeMemoryTests.RunAsync());
await results.RunAsync(() => NativeMemosTests.RunAsync());
await results.RunAsync(() => NativeHooksTests.RunAsync());
await results.RunAsync(() => NativeExampleTests.RunAsync());
await results.RunAsync(() => NativeSkillsTests.RunAsync());
results.Run(() => NativeSkillMarkdownTests.Run());
await results.RunAsync(() => NativeSkillSourceTests.RunAsync());
await results.RunAsync(() => NativeSkillStoreTests.RunAsync());
await results.RunAsync(() => NativePluginsTests.RunAsync());
await results.RunAsync(() => NativeBrowserSettingsTests.RunAsync());
await results.RunAsync(() => NativeConnectionSettingsTests.RunAsync());
await results.RunAsync(() => NativeNotificationSoundTests.RunAsync());
await results.RunAsync(() => NativeModelSettingsTests.RunAsync());
results.Run(() => NativeModelMediaTests.Run(args[0]));
await results.RunAsync(() => NativeProviderSettingsTests.RunAsync());
await results.RunAsync(() => NativeDecisionSettingsTests.RunAsync());
await results.RunAsync(() => NativeSubscriptionSettingsTests.RunAsync());
await results.RunAsync(() => NativeProxyRoutesTests.RunAsync());
await results.RunAsync(() => NativeChannelSettingsTests.RunAsync());
await results.RunAsync(() => NativeProjectSyncJoinTests.RunAsync());
results.ThrowIfFailed();
