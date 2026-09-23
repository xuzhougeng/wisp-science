using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

// Stand in for an incompatible desktop intercepting the settings-host launch.
if (args.SequenceEqual(new[] { "--native-settings-host" })) return;

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

var fixtureDirectory = Path.GetDirectoryName(args[0])!;
var sessions = JsonSerializer.Deserialize<ProjectBrowserResponse>(File.ReadAllText(Path.Combine(fixtureDirectory, "sessions.json")))!;
if (sessions.Type != "sessions" || sessions.Sessions?.Single().ProjectId != "research-1"
    || sessions.Sessions.Single().Status != "needs_you")
    throw new InvalidOperationException("Session DTO drift");
var transcript = JsonSerializer.Deserialize<ProjectBrowserResponse>(File.ReadAllText(Path.Combine(fixtureDirectory, "transcript.json")))!;
if (transcript.Type != "transcript" || transcript.Messages?.Count != 2
    || transcript.Messages[0].Sequence != 6 || transcript.NextBeforeSeq != 6)
    throw new InvalidOperationException("Transcript DTO drift");

await BrowserTests.RunAsync(fixtureDirectory);
var star = JsonSerializer.Deserialize<SetProjectStarredRequest>(File.ReadAllText(Path.Combine(fixtureDirectory, "set-project-starred.json")))!;
if (star.Schema != ProjectBrowserProtocol.Schema || star.Id != "projects-1"
    || star.Type != "set_project_starred" || star.ProjectId != "research-1" || !star.Starred)
    throw new InvalidOperationException("Project star command drift");
var encodedStar = JsonSerializer.SerializeToElement(star);
if (!encodedStar.GetProperty("starred").GetBoolean() || encodedStar.GetProperty("project_id").GetString() != "research-1")
    throw new InvalidOperationException("Project star serialization drift");

await NativeSettingsContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-settings/v1")));
await NativeProjectContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-projects/v1/create.json")));
await NativeLibraryContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-library/v1/search.json")));
await NativeCalendarContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-calendar/v1/month.json")));
await NativePrivacyContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-privacy/v1/mode.json")));
await NativeJourneyContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-journey/v1/range.json")));
await NativePublicationContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-publication/v1/workspace.json")));
await NativeScratchContractTests.Run(Path.GetFullPath(Path.Combine(fixtureDirectory, "../../native-scratch/v1/open.json")));
await NativeConversationContractTests.Run(args[0]);

NativePanelTabsTests.Run();
await AppearanceSettingsTests.RunAsync();
await WorkspaceActionTests.RunAsync();
await WorkspaceConversationTests.RunAsync(args[0]);
await NativeParityTests.RunAsync();
await NativeSettingsEditorTests.RunAsync();
await NativeModelSettingsTests.RunAsync();
await NativeChannelSettingsTests.RunAsync();
