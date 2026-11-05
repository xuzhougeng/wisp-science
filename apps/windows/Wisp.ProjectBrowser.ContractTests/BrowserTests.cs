using System.Diagnostics;
using System.Text.Json;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class BrowserTests
{
    private static int passed;
    private static void Check(bool value, string name)
    {
        if (!value) throw new InvalidOperationException(name);
        passed++;
        Console.WriteLine("PASS " + name);
    }

    private static async Task Throws<T>(Func<Task> action, string name) where T : Exception
    {
        try { await action(); } catch (T) { Check(true, name); return; }
        throw new InvalidOperationException("Expected " + typeof(T).Name + ": " + name);
    }

    public static async Task RunAsync(string fixtureDirectory)
    {
        await CreatedDraftAsync();
        await HomeActivityAsync();
        Check(PreviewLayout.InitialWindowSize(96, 1920, 1040, 16, 8) == (1116, 768) &&
            PreviewLayout.InitialWindowSize(144, 2560, 1500, 24, 12) == (1674, 1152),
            "initial client area stays 1100 by 760 DIPs across monitor scales, excluding the native frame");
        Check(PreviewLayout.InitialWindowSize(192, 1366, 728, 32, 16) == (1366, 728),
            "a high-DPI laptop with a smaller work area keeps the whole initial window reachable");
        var screenshotSize = PreviewLayout.ForSize(1200 / 1.5, 804 / 1.5);
        Check(screenshotSize.StackHomeColumns && screenshotSize.StackHeader && screenshotSize.ShortWindow,
            "150 percent scaling uses the WebView's narrow home layout in client DIPs");
        Check(PreviewLayout.ForSize(600, 500).StackHomeColumns, "small windows retain both home lists in the page scroller");
        Check(!PreviewLayout.ForSize(1200, 800).StackHeader && !PreviewLayout.ForSize(1200, 800).CompactWorkspace,
            "wide windows retain full action strip");
        var mixed = new BrowserMessage(1, "assistant", "先检查数据。\nrun_in_context\n{\"command\":\"first\\n第二行\"}\n检查完成。", null);
        var completion = new BrowserMessage(2, "assistant", "attempt_completion\n{\"result\":\"# 最终报告\\n已完成。\"}", null);
        var echo = new BrowserMessage(3, "tool", "# 最终报告\n已完成。", "attempt_completion");
        Check(TranscriptPresentation.Sections(completion).Single() is { ToolName: null, Text: "# 最终报告\n已完成。" }, "offline final report renders as Markdown body");
        Check(TranscriptPresentation.ReadableMessages([completion, echo]).Count == 1, "adjacent identical completion echo is displayed once");
        Check(TranscriptPresentation.ReadableMessages([completion with { Text = echo.Text }, echo]).Count == 2, "ordinary prose is not mistaken for a completion call");
        Check(TranscriptPresentation.ReadableMessages([completion, echo with { Text = "Error: failed" }]).Count == 2, "different completion output is not lost");
        Check(TranscriptPresentation.ReadableMessages([completion, mixed with { Role = "user" }, echo]).Count == 3, "completion echo never crosses user turn");
        Check(TranscriptPresentation.Sections(echo).Single().ToolName == null, "standalone final result stays visible");
        var toolCall = mixed with { Text = "read\n{\"path\":\"a.txt\"}" };
        var groups = TranscriptPresentation.Groups([toolCall, toolCall with { Sequence = 2 }, completion, echo]);
        Check(groups.Count == 2 && groups[0].IsProcess && groups[0].Messages.Count == 2 && !groups[1].IsProcess,
            "offline consecutive tool calls fold while final answer remains outside");
        Check(!TranscriptPresentation.Groups([echo with { Text = "Error: failed", ToolName = "read" }]).Single().IsProcess,
            "unknown tool success is not buried in a completed group");
        var sections = TranscriptPresentation.Sections(mixed);
        Check(sections.Count == 3 && sections[0].Text.Contains("先检查数据") && sections[2].Text.Contains("检查完成")
            && sections[1].ToolName == "run_in_context" && sections[1].Text.Contains("first\n第二行"),
            "tool arguments fold separately while prose and decoded Unicode/newlines remain readable");
        Check(TranscriptPresentation.Sections(mixed with { Role = "user" }).Single().ToolName == null,
            "user text resembling a tool call stays visible");
        Check(TranscriptPresentation.Sections(mixed with { Text = "function_name\n{broken JSON}" }).Single().ToolName == null,
            "malformed or ordinary function prose is never hidden as a tool call");
        Check(TranscriptPresentation.Sections(mixed with { Role = "tool", ToolName = "read_file", Text = "line1\nline2" }).Single()
            is { IsResult: true, Text: "line1\nline2", ToolName: "read_file" }, "tool result keeps its exact text and identity");
        var json = File.ReadAllText(Path.Combine(fixtureDirectory, "projects.json"));
        Check(ProjectBrowserClient.Decode(json, "projects-1", "projects").Projects!.Single().Name == "RNA-seq 研究", "shared fixture through production decoder");
        foreach (var invalid in new[] { json.Replace("wisp.project-browser.v1", "v2"), json.Replace("projects-1", "other"), json.Replace("persisted_only", "live") })
            await Throws<InvalidDataException>(() => Task.FromResult(ProjectBrowserClient.Decode(invalid, "projects-1", "projects")), "reject mismatched envelope");
        await Throws<InvalidDataException>(() => Task.FromResult(ProjectBrowserClient.Decode("{", "id", "projects")), "reject malformed JSON");
        await Throws<InvalidDataException>(() => Task.FromResult(ProjectBrowserClient.Decode(json, "projects-1", "transcript")), "reject wrong response type");

        var client = new ProjectBrowserClient(Environment.ProcessPath!, TimeSpan.FromSeconds(5));
        var temporary = Path.Combine(Path.GetTempPath(), "wisp winui 中文 " + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(temporary);
        try
        {
            var snapshot = await client.ListProjectsAsync(Path.Combine(temporary, "success"));
            Check(snapshot.Projects.Single().Name == "研究 中文", "UTF-8 transport and database argument with spaces");
            Check((await client.ListSessionsAsync(Path.Combine(temporary, "success"), "p")).Single().ProjectId == "p", "session command and project identity");
            Check((await client.SetProjectStarredAsync(Path.Combine(temporary, "success"), "p", true)).Projects.Single().Starred, "explicit writable process forwards desired star state");
            var page = await client.GetTranscriptAsync(Path.Combine(temporary, "success"), "p", "s", 8);
            Check(page.Messages.Single().Sequence == 7 && page.Messages.Single().Text.Length == 200000, "large stdout and stderr drain without deadlock; cursor forwarded");
            await Throws<InvalidOperationException>(() => client.ListProjectsAsync(Path.Combine(temporary, "error")), "service error envelope");
            await Throws<InvalidOperationException>(() => client.ListProjectsAsync(Path.Combine(temporary, "exit")), "nonzero service exit");
            await Throws<InvalidDataException>(() => client.ListProjectsAsync(Path.Combine(temporary, "empty")), "missing payload");
            var timeoutPath = Path.Combine(temporary, "hang");
            var timedClient = new ProjectBrowserClient(Environment.ProcessPath!, TimeSpan.FromMilliseconds(700));
            await Throws<TimeoutException>(() => timedClient.ListProjectsAsync(timeoutPath), "deadline ends hanging process");
            Check(!Alive(int.Parse(File.ReadAllText(timeoutPath + ".pid"))), "timed out process reaped");
            using var cancellation = new CancellationTokenSource(700);
            await Throws<OperationCanceledException>(() => client.ListProjectsAsync(timeoutPath, cancellation.Token), "caller cancellation propagated");
            Check(!Alive(int.Parse(File.ReadAllText(timeoutPath + ".pid"))), "cancelled process reaped");
        }
        finally { Directory.Delete(temporary, recursive: true); }

        var fake = new FakeClient();
        using var model = new ProjectBrowserModel(fake, "old");
        await model.RefreshAsync();
        Check(model.Projects.Count == 2 && model.RecentSessions.Count == 2, "refresh projects and recent sessions");
        model.ApplyPrivacy(new(true, ["p"]));
        Check(model.HomeProjects.Select(p => p.Id).SequenceEqual(["p2"]) && model.HomeRecentSessions.Select(s => s.Id).SequenceEqual(["s2"])
            && model.Search("").All(hit => hit.ProjectId == "p2"), "one selection conceals its project, recent conversations and search suggestions");
        model.ApplyPrivacy(null);
        Check(model.HomeProjects.Count == 0 && model.HomeRecentSessions.Count == 0, "unconfirmed privacy state never reveals cached rows");
        model.ApplyPrivacy(new(false, ["p"]));
        Check(model.HomeProjects.Count == 2 && model.HomeRecentSessions.Count == 2, "restore without reloading conversations");
        await model.SetStarredAsync("p", true);
        Check(model.Projects[0].Starred && model.ActiveProjectId is null, "star applies server snapshot without navigating");
        fake.Fail = true;
        await model.SetStarredAsync("p", false);
        Check(model.Projects[0].Starred && model.Error != null, "failed star retains previous snapshot");
        fake.Fail = false;
        await model.OpenProjectAsync("p", "s2");
        Check(model.ActiveSessionId == "s2" && model.Messages.Single().Text == "s2", "recent link selects exact session");
        var navigationEvents = 0;
        model.Changed += () => navigationEvents++;
        var messagesBeforeActivity = model.Messages;
        var live = new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", 1, "p", "s2", [], null,
            true, false, false, "m", null, null, [], ActivityStatus: "running");
        foreach (var status in new[] { "running", "needs_you", "complete" })
        {
            Check(model.ApplySessionActivity(live with { ActivityStatus = status })
                && model.Sessions.Single(s => s.Id == "s2").Status == status, "live sidebar status follows authoritative runtime transitions");
            Check(!model.ApplySessionActivity(live with { ActivityStatus = status }), "identical activity polls do not redraw the sidebar");
        }
        Check(navigationEvents == 0 && ReferenceEquals(messagesBeforeActivity, model.Messages) && model.ActiveSessionId == "s2",
            "activity updates preserve transcript, focus and navigation rather than refreshing the root");
        Check(!model.ApplySessionActivity(live with { ProjectId = "foreign" })
            && !model.ApplySessionActivity(live with { SessionId = "s1" })
            && !model.ApplySessionActivity(live with { ActivityStatus = null })
            && !model.ApplySessionActivity(live with { ActivityStatus = "unknown" }), "foreign, stale and older-host activity cannot overwrite sidebar state");
        Check(model.Search("").All(s => s.ProjectId == "p" && s.SessionId != null), "workspace search stays in current project");
        await model.OpenSessionAsync("s1");
        await model.OpenSessionAsync("s1", older: true);
        Check(model.Messages.Select(m => m.Sequence).SequenceEqual(new long[] { 1, 2, 3 }), "older page prepended in order without overlap");
        await model.OpenProjectAsync("p", "missing");
        Check(model.SessionError != null && model.Messages.Count == 0, "missing exact session never falls back to another conversation");
        fake.PendingSessions = new();
        var pending = model.OpenProjectAsync("p");
        model.GoHome();
        fake.PendingSessions.SetResult([new("s1", "p", "Session one", 0, "done")]);
        await pending;
        Check(model.ActiveProjectId == null && model.Sessions.Count == 0 && !model.SessionsLoading, "late project response cannot reopen home");
        fake.PendingSessions = null;
        await model.OpenProjectAsync("p");
        fake.PendingTranscript = new();
        pending = model.OpenSessionAsync("s2");
        model.GoHome();
        fake.PendingTranscript.SetResult(new([new(9, "assistant", "stale", null)], null));
        await pending;
        Check(model.Messages.Count == 0 && !model.TranscriptLoading, "late transcript ignored after navigation");
        fake.PendingTranscript = null;
        fake.PendingProjects = new();
        pending = model.RefreshAsync();
        var old = fake.PendingProjects;
        fake.PendingProjects = null;
        await model.ChangeDatabaseAsync("new");
        old.SetResult(new([Project("stale")], "persisted_only", new(false, [])));
        await pending;
        Check(model.DatabasePath == "new" && model.Projects.All(p => p.Id != "stale"), "database switch invalidates previous refresh");
        fake.Fail = true;
        await model.RefreshAsync();
        Check(model.Error != null && model.Projects.Count == 2 && model.LastLoaded != null, "failed refresh retains labelled previous snapshot");
        var reads = fake.ProjectReads;
        await model.RefreshAfterHostReadyAsync("new");
        Check(fake.ProjectReads == reads && model.Error != null && model.Projects.Count == 2,
            "host connection preserves previously loaded data and its refresh error");

        var firstLaunch = new FakeClient { Fail = true };
        using var fresh = new ProjectBrowserModel(firstLaunch, "first-run");
        await fresh.RefreshAsync();
        Check(fresh.Error != null && fresh.LastLoaded == null && fresh.Projects.Count == 0,
            "missing first-run database leaves a recoverable initial read error");
        firstLaunch.Fail = false;
        await fresh.RefreshAfterHostReadyAsync("other-database");
        Check(firstLaunch.ProjectReads == 1 && fresh.Error != null,
            "a different database's host cannot retry the initial read");
        firstLaunch.PendingProjects = new();
        var recovering = fresh.RefreshAfterHostReadyAsync("first-run");
        await fresh.RefreshAfterHostReadyAsync("first-run");
        Check(firstLaunch.ProjectReads == 2 && fresh.Loading,
            "duplicate host-ready callbacks do not overlap the recovery read");
        firstLaunch.PendingProjects.SetResult(new([Project("p")], "persisted_only", new(false, [])));
        await recovering;
        Check(fresh.Error == null && fresh.LastLoaded != null && fresh.Projects.Single().Id == "p"
            && fresh.RecentSessions.Count == 2 && !fresh.Loading,
            "host-ready recovery populates projects and recent sessions without manual refresh");
        await fresh.OpenProjectAsync("p", "s1");
        var restoredMessages = fresh.Messages;
        await fresh.RefreshAfterHostReadyAsync("first-run");
        Check(firstLaunch.ProjectReads == 2 && fresh.ActiveSessionId == "s1"
            && ReferenceEquals(fresh.Messages, restoredMessages),
            "later host connections preserve the selected session and transcript");

        var unavailable = new FakeClient { Fail = true };
        var joinedClient = new FakeClient();
        using var joinedBrowser = new ProjectBrowserModel(joinedClient, "joined-database");
        await joinedBrowser.RefreshAsync();
        await joinedBrowser.OpenProjectAsync("joined-project");
        Check(joinedBrowser.ActiveProjectId == null, "a newly joined project is not navigable from an old browser snapshot");
        joinedClient.PendingProjects = new();
        var joiningRefresh = joinedBrowser.RefreshAsync();
        joinedClient.PendingProjects.SetResult(new([Project("p"), Project("joined-project")], "persisted_only", new(false, [])));
        await joiningRefresh;
        await joinedBrowser.OpenProjectAsync("joined-project");
        Check(joinedBrowser.ActiveProjectId == "joined-project" && joinedBrowser.Projects.Any(p => p.Id == "joined-project")
            && joinedBrowser.Sessions.All(s => s.ProjectId == "joined-project"),
            "refresh after a successful join permits navigation to the returned project without reusing another project's sessions");
        using var failedRecovery = new ProjectBrowserModel(unavailable, "unavailable");
        await failedRecovery.RefreshAsync();
        await failedRecovery.RefreshAfterHostReadyAsync("unavailable");
        Check(unavailable.ProjectReads == 2 && failedRecovery.Error != null && !failedRecovery.Loading,
            "an unsuccessful recovery stays visible without an automatic retry loop");
        var disposed = new ProjectBrowserModel(unavailable, "closed");
        await disposed.RefreshAsync();
        disposed.Dispose();
        await disposed.RefreshAfterHostReadyAsync("closed");
        Check(unavailable.ProjectReads == 3, "host completion after window close cannot start another query");
        using var shelfBrowser = new ProjectBrowserModel(new FakeClient(), "synthetic");
        await shelfBrowser.RefreshAsync(); await shelfBrowser.OpenProjectAsync("p");
        await shelfBrowser.OpenShelvedSessionAsync("other", new("hidden", "Hidden title", 10));
        Check(shelfBrowser.ActiveSessionId != "hidden", "shelf navigation rejects a different active project");
        await shelfBrowser.OpenShelvedSessionAsync("p", new("hidden", "Hidden title", 10));
        Check(shelfBrowser.ActiveSessionId == "hidden" && shelfBrowser.ActiveSession?.Title == "Hidden title"
            && shelfBrowser.Messages.Single().Text == "hidden", "scoped shelf entry opens its transcript and title");
        Check(!shelfBrowser.Sessions.Any(s => s.Id == "hidden") && !shelfBrowser.Search("Hidden title").Any(), "opening preserves shelved visibility");
        await shelfBrowser.RefreshAsync();
        Check(shelfBrowser.ActiveSessionId == "hidden" && shelfBrowser.SessionError == null, "refresh retains the open shelved conversation");
        await shelfBrowser.OpenSessionAsync("hidden", older: true);
        Check(shelfBrowser.Messages.Count == 3, "shelved transcript supports older pages");
        shelfBrowser.GoHome();
        Check(shelfBrowser.ActiveSession == null, "home clears the detached shelf entry");
        await shelfBrowser.OpenProjectAsync("p", selectFirst: false);
        Check(shelfBrowser.ActiveSessionId == null && shelfBrowser.Messages.Count == 0 && shelfBrowser.Sessions.Count > 0, "deletion refresh keeps the blank conversation instead of opening a neighbor");
        await shelfBrowser.OpenProjectAsync("p", "removed", selectFirst: false);
        Check(shelfBrowser.ActiveSessionId == null && shelfBrowser.SessionError == null, "unknown deletion acknowledgement reconciles a missing active conversation");
        var shelvingClient = new FakeClient(); using var shelvingBrowser = new ProjectBrowserModel(shelvingClient, "synthetic");
        await shelvingBrowser.RefreshAsync(); await shelvingBrowser.OpenProjectAsync("p", "s1");
        var current = shelvingBrowser.ActiveSession!;
        shelvingClient.PendingSessions = new(); shelvingClient.PendingSessions.SetResult([]);
        await shelvingBrowser.RefreshAfterShelvingAsync(current);
        Check(shelvingBrowser.ActiveSessionId == "s1" && shelvingBrowser.ActiveSession?.Title == current.Title
            && shelvingBrowser.Messages.Single().Text == "s1" && shelvingBrowser.Sessions.Count == 0 && shelvingBrowser.SessionError == null, "shelving current conversation keeps its transcript and metadata");
        Check(!shelvingBrowser.RecentSessions.Any(s => s.Id == "s1") && !shelvingBrowser.Search(current.Title).Any(), "shelving also removes cached recent/default-search entries");
        await shelvingBrowser.RefreshAfterShelvingAsync(current with { ProjectId = "other" });
        Check(shelvingBrowser.ActiveProjectId == "p", "late shelf result cannot navigate into another project");
        using var transferred = new ProjectBrowserModel(new FakeClient(), "synthetic");
        await transferred.RefreshAsync(); await transferred.OpenProjectAsync("p", "s1");
        var movedSource = transferred.ActiveSession!;
        await transferred.RefreshAfterTransferAsync(movedSource, moved: false);
        Check(transferred.ActiveSessionId == "s1" && transferred.Messages.Count > 0, "copy keeps the source conversation open");
        await transferred.RefreshAfterTransferAsync(movedSource, moved: true);
        Check(transferred.ActiveProjectId == "p" && transferred.ActiveSessionId == null && transferred.Messages.Count == 0
            && transferred.Sessions.Count > 0, "moving active conversation stays in source project without opening a neighbor");
        await transferred.OpenProjectAsync("p", "s2");
        await transferred.RefreshAfterTransferAsync(movedSource, moved: true);
        Check(transferred.ActiveSessionId == "s2", "moving a different conversation retains current reading");
        await transferred.RefreshAfterTransferAsync(movedSource with { ProjectId = "other" }, moved: true);
        Check(transferred.ActiveProjectId == "p" && transferred.ActiveSessionId == "s2", "late transfer cannot change another project");
        var racingClient = new FakeClient(); using var racingTransfer = new ProjectBrowserModel(racingClient, "synthetic");
        await racingTransfer.RefreshAsync(); await racingTransfer.OpenProjectAsync("p", "s1");
        var racingSource = racingTransfer.ActiveSession!; racingClient.PendingProjects = new();
        var olderRefresh = racingTransfer.RefreshAsync();
        await racingTransfer.RefreshAfterTransferAsync(racingSource, moved: true);
        racingClient.PendingProjects.SetResult(new([Project("p"), Project("p2")], "persisted_only", new(false, []))); await olderRefresh;
        Check(racingTransfer.ActiveSessionId == null && racingTransfer.Messages.Count == 0, "older in-flight refresh cannot reopen a neighbor after move");
        Console.WriteLine($"{passed} Windows client / navigation checks passed.");
    }

    private static bool Alive(int pid)
    {
        try { using var p = Process.GetProcessById(pid); return !p.HasExited; } catch (ArgumentException) { return false; }
    }
    private static ProjectSummary Project(string id) => new(id, id, "", "C:\\research", false, 2, 0, 0, 0, 0, false, null);

    // This executable doubles as a fake JSONL subprocess; tests require no database or installed runtime service.
    public static async Task FakeServiceAsync(string database)
    {
        Console.InputEncoding = System.Text.Encoding.UTF8;
        Console.OutputEncoding = new System.Text.UTF8Encoding(false);
        var request = JsonDocument.Parse((await Console.In.ReadLineAsync())!);
        var root = request.RootElement;
        var id = root.GetProperty("id").GetString();
        var mode = Path.GetFileName(database);
        if (mode == "hang") { File.WriteAllText(database + ".pid", Environment.ProcessId.ToString()); await Task.Delay(30000); return; }
        if (mode == "exit") { Console.Error.WriteLine("fake unavailable database"); Environment.Exit(4); }
        if (mode == "error") { Console.WriteLine(JsonSerializer.Serialize(new { schema = ProjectBrowserProtocol.Schema, id, type = "error", message = "fake query failed" })); return; }
        if (mode == "empty") { Console.WriteLine(JsonSerializer.Serialize(new { schema = ProjectBrowserProtocol.Schema, id, type = "projects", activity_source = "persisted_only" })); return; }
        var type = root.GetProperty("type").GetString();
        object reply = type switch
        {
            "set_project_starred" when Environment.GetCommandLineArgs().Contains("--allow-project-writes") && root.GetProperty("project_id").GetString() == "p" => new { schema = ProjectBrowserProtocol.Schema, id, type = "projects", activity_source = "persisted_only", privacy = new { active = false, project_ids = Array.Empty<string>() }, projects = new[] { Project("p") with { Starred = root.GetProperty("starred").GetBoolean() } } },
            "list_projects" => new { schema = ProjectBrowserProtocol.Schema, id, type = "projects", activity_source = "persisted_only", privacy = new { active = false, project_ids = Array.Empty<string>() }, projects = new[] { Project("p") with { Name = "研究 中文" } } },
            "list_sessions" => new { schema = ProjectBrowserProtocol.Schema, id, type = "sessions", activity_source = "persisted_only", sessions = new[] { new BrowserSession("s", root.GetProperty("project_id").GetString()!, "标题", 0, "done") } },
            _ => new { schema = ProjectBrowserProtocol.Schema, id, type = "transcript", messages = new[] { new BrowserMessage(root.GetProperty("before_seq").GetInt64() - 1, "assistant", new string('文', 200000), null) } }
        };
        await Console.Error.WriteAsync(new string('x', 40000));
        Console.WriteLine(JsonSerializer.Serialize(reply));
    }

    private static async Task CreatedDraftAsync()
    {
        var client = new FakeClient { UnlistedTranscript = "confirmed-empty" }; using var model = new ProjectBrowserModel(client, "memory");
        await model.RefreshAsync(); await model.OpenProjectAsync("p", selectFirst: false);
        await model.OpenCreatedSessionAsync("p", "confirmed-empty");
        Check(model.ActiveSessionId == "confirmed-empty" && model.ActiveSession?.Title == "" && model.SessionError == null
            && model.Sessions.Any(s => s.Id == "confirmed-empty"), "acknowledged empty frame is visible even when persisted history omits it");
        await model.RefreshAsync();
        Check(model.ActiveSessionId == "confirmed-empty" && model.SessionError == null, "list refresh retains the active empty draft");
        Check(client.TranscriptReads == 0 && model.Messages.Count == 0 && !model.TranscriptLoading,
            "empty frame adoption and refresh never call the persisted-only transcript endpoint");
        await model.OpenCreatedSessionAsync("p2", "wrong-project");
        Check(model.ActiveSessionId == "confirmed-empty", "creation adoption cannot cross projects");
        await model.OpenSessionAsync("s1");
        Check(!model.Sessions.Any(s => s.Id == "confirmed-empty"), "leaving the empty frame removes its temporary history row");
        await model.OpenCreatedSessionAsync("p", "confirmed-empty");
        client.UnlistedTranscript = null;
        client.PendingSessions = new(); client.PendingSessions.SetResult([new("confirmed-empty", "p", "Saved title", 10, "complete")]);
        await model.OpenProjectAsync("p", "confirmed-empty");
        Check(model.ActiveSession?.Title == "Saved title" && model.Sessions.Count == 1, "persisted row replaces the temporary draft instead of duplicating it");
        Check(model.Messages.Single().Text == "confirmed-empty", "saved frames resume ordinary persisted transcript reads");
        model.GoHome(); client.PendingSessions = null; await model.OpenProjectAsync("p", selectFirst: false);
        Check(!model.Sessions.Any(s => s.Id == "confirmed-empty"), "home clears temporary draft history state");
    }

    private static async Task HomeActivityAsync()
    {
        using var model = new ProjectBrowserModel(new FakeClient(), "scope");
        await model.RefreshAsync();
        var persisted = model.HomeRecentSessions.ToArray(); var updates = 0;
        model.HomeActivityChanged += () => updates++;
        var live = new HomeActivitySnapshot("rev", new(false, []), [new("p", 2, 0)],
            [persisted[0] with { Status = "running" }, persisted[1]]);
        model.ApplyHomeActivity(live); model.ApplyHomeActivity(live);
        Check(updates == 1 && model.HomeProjects[0].RunningCount == 2 && model.HomeRecentSessions[0].Status == "running",
            "two project turns can be running even when only one of them is recent; an unchanged snapshot does not redraw");
        model.ApplyHomeActivity(null);
        Check(model.HomeActivitySavedOnly && model.HomeRecentSessions.SequenceEqual(persisted) && model.HomeProjects[0].RunningCount == 0,
            "loss of the host restores saved labels and counts");
        model.ApplyHomeActivity(live with { Privacy = new(true, ["p"]) });
        Check(!model.HomeActivitySavedOnly && model.HomeProjects.All(p => p.Id != "p") && model.HomeRecentSessions.All(s => s.ProjectId != "p"),
            "project and recent columns share the same visibility even for stale or malformed hidden entries");
        model.ApplyPrivacy(new(false, [])); await model.OpenProjectAsync("p", "s1");
        model.ApplyHomeActivity(live);
        Check(model.ActiveSessionId == "s1" && model.HomeRecentSessions.SequenceEqual(persisted), "late home reads cannot change the workspace");
    }

    private sealed class FakeClient : IProjectBrowserClient
    {
        public Task<ProjectListSnapshot> SetProjectStarredAsync(string databasePath, string projectId, bool starred, CancellationToken cancellationToken = default) => Fail ? Task.FromException<ProjectListSnapshot>(new IOException("star failed")) : PendingProjects?.Task ?? Task.FromResult(new ProjectListSnapshot([Project(projectId) with { Starred = starred }, Project("p2")], "persisted_only", new(false, [])));
        public TaskCompletionSource<IReadOnlyList<BrowserSession>>? PendingSessions;
        public TaskCompletionSource<TranscriptPage>? PendingTranscript;
        public TaskCompletionSource<ProjectListSnapshot>? PendingProjects;
        public bool Fail;
        public int ProjectReads;
        public int TranscriptReads;
        public string? UnlistedTranscript;
        public Task<ProjectListSnapshot> ListProjectsAsync(string path, CancellationToken cancellationToken = default)
        {
            ProjectReads++;
            return Fail ? Task.FromException<ProjectListSnapshot>(new IOException("unavailable"))
                : PendingProjects?.Task ?? Task.FromResult(new ProjectListSnapshot([Project("p"), Project("p2")], "persisted_only", new(false, [])));
        }
        public Task<IReadOnlyList<BrowserSession>> ListSessionsAsync(string path, string? projectId = null, CancellationToken cancellationToken = default) =>
            PendingSessions?.Task ?? Task.FromResult<IReadOnlyList<BrowserSession>>([new("s1", projectId ?? "p", "Session one", 0, "done"), new("s2", projectId ?? "p2", "Session two", 0, "needs_you")]);
        public Task<TranscriptPage> GetTranscriptAsync(string path, string projectId, string sessionId, long? beforeSeq = null, CancellationToken cancellationToken = default)
        {
            TranscriptReads++;
            if (sessionId == UnlistedTranscript) return Task.FromException<TranscriptPage>(new InvalidOperationException("Session not found in project"));
            return PendingTranscript?.Task ?? Task.FromResult(beforeSeq == null ? new TranscriptPage([new(3, "assistant", sessionId, null)], 3)
                : new TranscriptPage([new(1, "user", "first", null), new(2, "assistant", "second", null), new(3, "assistant", sessionId, null)], null));
        }
    }
}
