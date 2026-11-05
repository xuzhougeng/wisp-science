using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeExplorationTests
{
    public static async Task RunAsync(string fixtures)
    {
        var data = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtures, "../../native-conversations/v1/explorations.json")))!;
        var rows = NativeExplorations.Decode(data, "research-1").Explorations;
        Check(rows[0].FullyIsolated && !rows[1].FullyIsolated, "shared full/partial isolation");
        Check(!(rows[0] with { Exploration = rows[0].Exploration with { WarningsJson = "[\"warning\"]" } }).FullyIsolated, "warnings prevent claiming complete isolation");
        Check((rows[0] with { IsolationSummaryJson = "bad", Exploration = rows[0].Exploration with { WarningsJson = "bad" } }).FullyIsolated, "legacy metadata matches WebView fallback");
        foreach (var mode in new[] { "project", "duplicate", "missing_checkpoint", "source" })
        {
            var invalid = data.DeepClone();
            if (mode == "project") invalid["project_id"] = "other";
            if (mode == "duplicate") invalid["explorations"]!.AsArray().Add(invalid["explorations"]![0]!.DeepClone());
            if (mode == "missing_checkpoint") invalid["explorations"]![0]!.AsObject().Remove("checkpoint_user_index");
            if (mode == "source") invalid["explorations"]![0]!["source_frame_id"] = "private-1";
            Reject(() => NativeExplorations.Decode(invalid, "research-1"));
        }
        var fake = new Host(data);
        using var groups = new WorkspaceSessionGroups(fake, "research-1");
        Check(await groups.LoadExplorationsAsync() && groups.Explorations.Length == 2, "scoped groups load");
        fake.Pending = new(); using var closedGroups = new WorkspaceSessionGroups(fake, "research-1");
        var pendingList = closedGroups.LoadExplorationsAsync(); closedGroups.Dispose(); fake.Pending.SetResult(data.DeepClone());
        Check(!await pendingList && closedGroups.Explorations.Length == 0, "closed project ignores late exploration list");
        fake.Pending = null;
        var nesting = new NativeSessionNesting();
        Check(nesting.Expanded("main-1", rows.Select(row => row.Session("research-1")), "private-1"), "active exploration expands source");
        nesting.SetExpanded("main-1", false);
        Check(!nesting.Expanded("main-1", rows.Select(row => row.Session("research-1")), "private-1"), "explicit collapse survives active exploration");
        var browser = new Browser(JsonSerializer.Deserialize<ProjectBrowserResponse>(File.ReadAllText(Path.Combine(fixtures, "projects.json")))!.Projects!.Single());
        using var model = new ProjectBrowserModel(browser, "synthetic");
        await model.RefreshAsync(); await model.OpenProjectAsync("research-1", selectFirst: false);
        Check(await model.OpenExplorationAsync(fake, "research-1", "explore-1", "private-1"), "validated exploration opens");
        Check(model.ActiveSession?.Id == "private-1" && model.Sessions.All(row => row.Id != "private-1")
            && model.Search("Alternative").Count() == 0 && browser.Transcripts == 0, "private metadata stays out of ordinary lists/search/transcript API");
        await model.RefreshAsync(selectFirst: false);
        Check(model.ActiveSession?.Id == "private-1" && browser.Transcripts == 0, "refresh preserves private navigation without browser transcript read");
        var intent = model.SessionNavigationRevision;
        await model.OpenSessionAsync("main-1"); Check(model.ActiveSession?.Id == "main-1" && browser.Transcripts == 1
            && model.SessionNavigationRevision != intent, "normal navigation invalidates a host-connect intent and returns to ordinary transcript");
        fake.Pending = new(); var pending = model.OpenExplorationAsync(fake, "research-1", "explore-1", "private-1");
        await model.OpenSessionAsync("main-1"); fake.Pending.SetResult(fake.Open());
        Check(!await pending && model.ActiveSession?.Id == "main-1", "late exploration cannot overwrite later ordinary navigation");
        fake.Pending = new(); pending = model.OpenExplorationAsync(fake, "research-1", "explore-1", "private-1");
        model.GoHome(); fake.Pending.SetResult(fake.Open());
        Check(!await pending && model.ActiveProjectId == null, "late exploration cannot leave home");
        fake.Pending = null; var wrongOpen = fake.Open(); wrongOpen["summary"]!["exploration"]!["id"] = "other";
        Reject(() => NativeExplorationOpen.Decode(wrongOpen, "research-1", "explore-1", "private-1"));
        var snapshot = new ConversationSnapshot(ConversationSnapshot.SchemaId, "test", 1, "research-1", "main-1", [], null,
            false, false, true, "", null, null, [], Exploration: new(null, 1, true));
        var frozen = JsonSerializer.SerializeToNode(snapshot, ConversationSnapshot.JsonOptions)!;
        Check(ConversationSnapshot.Decode(frozen, "research-1", "main-1").ReadOnly, "source freeze round-trips");
        frozen["read_only"] = false; Reject(() => ConversationSnapshot.Decode(frozen, "research-1", "main-1"));
        var privateSnapshot = snapshot with { SessionId = "private-2", Exploration = new(rows[1], 0, false), ReadOnly = false };
        Reject(() => ConversationSnapshot.Decode(JsonSerializer.SerializeToNode(privateSnapshot, ConversationSnapshot.JsonOptions), "research-1", "private-2"));
        Check(fake.Calls.All(call => call == "research-1"), "every host call carries explicit project scope");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static void Reject(Action action) { try { action(); } catch (Exception e) when (e is InvalidDataException or JsonException) { return; } throw new InvalidOperationException("Invalid exploration accepted"); }
    private sealed class Host(JsonNode data) : INativeSettingsClient
    {
        public TaskCompletionSource<JsonNode?>? Pending; public List<string?> Calls = [];
        public JsonNode Open() => new JsonObject { ["project_id"] = "research-1", ["summary"] = data["explorations"]![0]!.DeepClone() };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add(projectId);
            return Pending?.Task ?? Task.FromResult<JsonNode?>(command == "native_conversation_explorations" ? data.DeepClone() : Open());
        }
    }
    private sealed class Browser(ProjectSummary project) : IProjectBrowserClient
    {
        public int Transcripts;
        public Task<ProjectListSnapshot> ListProjectsAsync(string databasePath, CancellationToken cancellationToken = default) => Task.FromResult(new ProjectListSnapshot([project], "persisted", new(false, [])));
        public Task<ProjectListSnapshot> SetProjectStarredAsync(string databasePath, string projectId, bool starred, CancellationToken cancellationToken = default) => throw new NotSupportedException();
        public Task<IReadOnlyList<BrowserSession>> ListSessionsAsync(string databasePath, string? projectId = null, CancellationToken cancellationToken = default) => Task.FromResult<IReadOnlyList<BrowserSession>>([new("main-1", "research-1", "Main", 0, "complete")]);
        public Task<TranscriptPage> GetTranscriptAsync(string databasePath, string projectId, string sessionId, long? beforeSeq = null, CancellationToken cancellationToken = default)
        { if (sessionId != "main-1") throw new InvalidOperationException("Private frame reached ordinary API"); Transcripts++; return Task.FromResult(new TranscriptPage([], null)); }
    }
}
