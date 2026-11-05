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
await HomeProjectSyncTests.RunAsync();
NativeAcpChecksTests.Run();

var fixtureDirectory = Path.GetDirectoryName(args[0])!;
await HomeActivityContractTests.RunAsync(Path.Combine(fixtureDirectory, "../../native-projects/v1/home-activity.json"));
var sessions = JsonSerializer.Deserialize<ProjectBrowserResponse>(File.ReadAllText(Path.Combine(fixtureDirectory, "sessions.json")))!;
if (sessions.Type != "sessions" || sessions.Sessions?.Single().ProjectId != "research-1"
    || sessions.Sessions.Single().Status != "needs_you")
    throw new InvalidOperationException("Session DTO drift");
var transcript = JsonSerializer.Deserialize<ProjectBrowserResponse>(File.ReadAllText(Path.Combine(fixtureDirectory, "transcript.json")))!;
if (transcript.Type != "transcript" || transcript.Messages?.Count != 2
    || transcript.Messages[0].Sequence != 6 || transcript.NextBeforeSeq != 6)
    throw new InvalidOperationException("Transcript DTO drift");

await BrowserTests.RunAsync(fixtureDirectory);
await ProjectChatHistoryTests.RunAsync(Path.Combine(fixtureDirectory, "chat-matches.json"));
var star = JsonSerializer.Deserialize<SetProjectStarredRequest>(File.ReadAllText(Path.Combine(fixtureDirectory, "set-project-starred.json")))!;
if (star.Schema != ProjectBrowserProtocol.Schema || star.Id != "projects-1"
    || star.Type != "set_project_starred" || star.ProjectId != "research-1" || !star.Starred)
    throw new InvalidOperationException("Project star command drift");
var encodedStar = JsonSerializer.SerializeToElement(star);
if (!encodedStar.GetProperty("starred").GetBoolean() || encodedStar.GetProperty("project_id").GetString() != "research-1")
    throw new InvalidOperationException("Project star serialization drift");

await NativeSettingsContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-settings/v1")));
await NativeShareExportTests.RunAsync();
await NativeFilesTests.RunAsync(fixtureDirectory);
await NativeFileSelectionTests.RunAsync();
await NativeProjectFilesTests.RunAsync();
await NativeAttachmentTests.RunAsync();
await NativeClipboardTests.RunAsync();
await NativeSidebarReferenceTests.RunAsync();
await NativeAcpConversationTests.RunAsync();
await NativeAcpSelectionTests.RunAsync();
await NativeDraftModelSelectionTests.RunAsync();
await NativeModelCompactionTests.RunAsync();
await NativeCompactionHistoryTests.RunAsync();
NativeAcpQuestionTests.Run();
NativeComposerMenuTests.Run();
NativeComposerModelMenuTests.Run();
        NativeComputeMenuTests.Run();
await NativeConversationHistoryTests.RunAsync();
await NativeConversationExcerptTests.RunAsync();
await NativeTranscriptSelectionTests.RunAsync();
NativeSourceSelectionTests.Run();
NativePreviewSelectionTests.Run();
await NativeReviewPresentationTests.RunAsync();
await NativeConversationQueueTests.RunAsync();
await NativeExecutionReadingTests.RunAsync();
await NativePlanProposalTests.RunAsync();
await NativeHostReconnectTests.RunAsync();
await NativeSearchTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-search/v1/search.json")));
await NativeSessionImportTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-session-import/v1/archive.json")));
await NativeExternalImportTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-session-import/v1/external.json")));
await NativeProjectContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-projects/v1/create.json")));
await NativeLibraryContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-library/v1/search.json")));
await NativeCalendarContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-calendar/v1/month.json")));
await NativeCalendarPresentationTests.RunAsync();
await NativeLibraryPresentationTests.Run();
await NativePrivacyContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-privacy/v1/mode.json")));
await NativeJourneyContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-journey/v1/range.json")));
await NativePublicationContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-publication/v1/workspace.json")));
await NativeConversationContractTests.Run(args[0]);

NativePanelTabsTests.Run();
NativeSessionRunGroupsTests.Run();
await NativeArtifactModalTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-conversations/v1/artifact-provenance.json")));
await NativeFileProvenanceTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-conversations/v1/file-provenance.json")));
NativeCodePreviewTests.Run();
NativeArtifactLayoutTests.Run();
NativeWorkspaceLayoutTests.Run();
NativeHomeLayoutTests.Run();
NativeProjectDialogLayoutTests.Run();
NativePanelViewStateTests.Run();
NativeFileListPresentationTests.Run();
NativeActionLayoutTests.Run();
await NativeSideChatPresentationTests.RunAsync();
NativeToolPresentationTests.Run();
NativeSettingsSearchTests.Run();
NativeSettingsLayoutTests.Run();
NativePreferenceIntegerTests.Run();
await NativeStoragePresentationTests.RunAsync();
await NativeUsageTests.RunAsync();
await NativeProjectSettingsTests.RunAsync();
await NativeProjectImportTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-projects/v1/recovery.json")));
await NativeProjectExportTests.RunAsync();
await NativeProjectDeleteTests.RunAsync(fixtureDirectory);
await NativeWindowMenuTests.RunAsync(fixtureDirectory);
await NativeUpdateDialogTests.RunAsync();
NativeLocalizationTests.Run();
await NativeInlineSettingsTests.RunAsync();
await NativeLocalEnvironmentTests.RunAsync();
await NativeQuickSetupTests.RunAsync();
await NativeProjectInventoryTests.RunAsync();
await NativeNetworkSettingsTests.RunAsync();
await NativeAssistantTests.RunAsync();
await NativeComposeMenuTests.RunAsync();
await NativeAssistantRemoteTests.RunAsync();
await NativeAutomationTests.RunAsync();
await NativeMandateTests.RunAsync(args[0]);
await NativePetPresentationTests.RunAsync();
await NativeWorkflowStudioTests.RunAsync();
await NativeWorkflowOptionsTests.RunAsync();
await NativeWorkflowConversionTests.RunAsync();
NativeComposerCommandsTests.Run();
await NativeComposerReferencesTests.RunAsync(args[0]);
NativeTranscriptRowsTests.Run();
await NativeErrorResumeTests.RunAsync();
NativeTranscriptActivityTests.Run();
NativeTranscriptTablesTests.Run();
NativeCsvTableTests.Run();
await NativeArtifactActionTests.RunAsync();
await NativeArtifactQuoteTests.RunAsync();
await NativeGeneratedQuoteTests.RunAsync();
await NativeArtifactLibraryTests.RunAsync();
await NativeTerminalStreamingTests.RunAsync();
await WorkspaceJourneyTests.RunAsync();
await NativeJourneyPresentationTests.RunAsync();
NativeBrowserPresentationTests.Run();
await NativeComposerEffortTests.RunAsync();
await NativeRuntimeStripTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-runtimes/v1/snapshot.json")));
await NativeRuntimeInterpreterTests.RunAsync();
await NativeRuntimeInspectorTests.RunAsync();
await NativeComposerOptionsTests.RunAsync(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-conversations/v1/composer-options.json")));
NativeTranscriptTests.Run();
await NativeWorkspaceActionsTests.RunAsync();
await AppearanceSettingsTests.RunAsync();
NativeMessagePresentationTests.Run();
await NativeGeneratedArtifactsTests.RunAsync();
await NativeArtifactCatalogTests.RunAsync();
await NativeArtifactFilesTests.RunAsync();
await NativeLiveAppearanceTests.RunAsync();
await NativeCustomThemeTests.RunAsync();
await WorkspaceActionTests.RunAsync();
await NativeShelvedSessionsTests.RunAsync(fixtureDirectory);
await NativeSessionDeletionTests.RunAsync();
await NativeSessionMenusTests.RunAsync();
NativeSessionTreeTests.Run(fixtureDirectory);
await NativeSessionGroupingTests.RunAsync();
await NativeFolderActionsTests.RunAsync(fixtureDirectory);
await NativeBranchMergeTests.RunAsync(fixtureDirectory);
NativeBranchResultsTests.Run(fixtureDirectory);
await NativeSessionRulesTests.RunAsync(fixtureDirectory);
await NativeExplorationTests.RunAsync(fixtureDirectory);
await NativeExplorationPreviewTests.RunAsync(fixtureDirectory);
await NativeExplorationStartTests.RunAsync(fixtureDirectory);
await NativeSessionTransferTests.RunAsync(fixtureDirectory);
await NativeDebugRequestExportTests.RunAsync(fixtureDirectory);
await NativeRunNavigationTests.RunAsync();
await NativeMethodReviewTests.RunAsync(Path.Combine(fixtureDirectory, "../../native-runtimes/v1/method-search.json"));
await NativeRunReviewTests.RunAsync(args[0]);
await WorkspaceConversationTests.RunAsync(args[0]);
await NativeParityTests.RunAsync();
await NativeContextDetailsTests.RunAsync();
NativeContextHealthTests.Run();
await NativeHandoffTests.RunAsync();
await NativeIdleCompactionTests.RunAsync();
await NativeContextRecoveryTests.RunAsync();
NativeContextPanelLayoutTests.Run();
await NativeModelContextTests.RunAsync();
await NativeSettingsEditorTests.RunAsync();
await NativePermissionTests.RunAsync();
await NativeCredentialTests.RunAsync();
await NativeEnvironmentTests.RunAsync();
await NativeReviewerTests.RunAsync();
await NativeSpecialistTransferTests.RunAsync();
NativeWindowEscapeTests.Run();
await NativeMemoryTests.RunAsync();
await NativeMemosTests.RunAsync();
await NativeHooksTests.RunAsync();
await NativeExampleTests.RunAsync();
await NativeSkillsTests.RunAsync();
NativeSkillMarkdownTests.Run();
await NativeSkillSourceTests.RunAsync();
await NativeSkillStoreTests.RunAsync();
await NativePluginsTests.RunAsync();
await NativeBrowserSettingsTests.RunAsync();
await NativeConnectionSettingsTests.RunAsync();
await NativeNotificationSoundTests.RunAsync();
await NativeModelSettingsTests.RunAsync();
NativeModelMediaTests.Run(args[0]);
await NativeProviderSettingsTests.RunAsync();
await NativeDecisionSettingsTests.RunAsync();
await NativeSubscriptionSettingsTests.RunAsync();
await NativeProxyRoutesTests.RunAsync();
await NativeChannelSettingsTests.RunAsync();
await NativeProjectSyncJoinTests.RunAsync();
