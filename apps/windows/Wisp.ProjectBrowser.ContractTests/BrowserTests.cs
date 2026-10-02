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
        var screenshotSize = PreviewLayout.ForSize(1200 / 1.5, 804 / 1.5);
        Check(!screenshotSize.StackHomeColumns && screenshotSize.StackHeader && screenshotSize.ShortWindow,
            "150 percent scaling keeps recent sessions beside projects and bounds sidebar tools");
        Check(PreviewLayout.ForSize(600, 500).StackHomeColumns, "small windows retain both home lists in stacked scroll regions");
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
        await model.SetStarredAsync("p", true);
        Check(model.Projects[0].Starred && model.ActiveProjectId is null, "star applies server snapshot without navigating");
        fake.Fail = true;
        await model.SetStarredAsync("p", false);
        Check(model.Projects[0].Starred && model.Error != null, "failed star retains previous snapshot");
        fake.Fail = false;
        await model.OpenProjectAsync("p", "s2");
        Check(model.ActiveSessionId == "s2" && model.Messages.Single().Text == "s2", "recent link selects exact session");
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
        old.SetResult(new([Project("stale")], "persisted_only"));
        await pending;
        Check(model.DatabasePath == "new" && model.Projects.All(p => p.Id != "stale"), "database switch invalidates previous refresh");
        fake.Fail = true;
        await model.RefreshAsync();
        Check(model.Error != null && model.Projects.Count == 2 && model.LastLoaded != null, "failed refresh retains labelled previous snapshot");
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
            "set_project_starred" when Environment.GetCommandLineArgs().Contains("--allow-project-writes") && root.GetProperty("project_id").GetString() == "p" => new { schema = ProjectBrowserProtocol.Schema, id, type = "projects", activity_source = "persisted_only", projects = new[] { Project("p") with { Starred = root.GetProperty("starred").GetBoolean() } } },
            "list_projects" => new { schema = ProjectBrowserProtocol.Schema, id, type = "projects", activity_source = "persisted_only", projects = new[] { Project("p") with { Name = "研究 中文" } } },
            "list_sessions" => new { schema = ProjectBrowserProtocol.Schema, id, type = "sessions", activity_source = "persisted_only", sessions = new[] { new BrowserSession("s", root.GetProperty("project_id").GetString()!, "标题", 0, "done") } },
            _ => new { schema = ProjectBrowserProtocol.Schema, id, type = "transcript", messages = new[] { new BrowserMessage(root.GetProperty("before_seq").GetInt64() - 1, "assistant", new string('文', 200000), null) } }
        };
        await Console.Error.WriteAsync(new string('x', 40000));
        Console.WriteLine(JsonSerializer.Serialize(reply));
    }

    private sealed class FakeClient : IProjectBrowserClient
    {
        public Task<ProjectListSnapshot> SetProjectStarredAsync(string databasePath, string projectId, bool starred, CancellationToken cancellationToken = default) => Fail ? Task.FromException<ProjectListSnapshot>(new IOException("star failed")) : PendingProjects?.Task ?? Task.FromResult(new ProjectListSnapshot([Project(projectId) with { Starred = starred }, Project("p2")], "persisted_only"));
        public TaskCompletionSource<IReadOnlyList<BrowserSession>>? PendingSessions;
        public TaskCompletionSource<TranscriptPage>? PendingTranscript;
        public TaskCompletionSource<ProjectListSnapshot>? PendingProjects;
        public bool Fail;
        public Task<ProjectListSnapshot> ListProjectsAsync(string path, CancellationToken cancellationToken = default) => Fail
            ? Task.FromException<ProjectListSnapshot>(new IOException("unavailable"))
            : PendingProjects?.Task ?? Task.FromResult(new ProjectListSnapshot([Project("p"), Project("p2")], "persisted_only"));
        public Task<IReadOnlyList<BrowserSession>> ListSessionsAsync(string path, string? projectId = null, CancellationToken cancellationToken = default) =>
            PendingSessions?.Task ?? Task.FromResult<IReadOnlyList<BrowserSession>>([new("s1", projectId ?? "p", "Session one", 0, "done"), new("s2", projectId ?? "p2", "Session two", 0, "needs_you")]);
        public Task<TranscriptPage> GetTranscriptAsync(string path, string projectId, string sessionId, long? beforeSeq = null, CancellationToken cancellationToken = default) =>
            PendingTranscript?.Task ?? Task.FromResult(beforeSeq == null ? new TranscriptPage([new(3, "assistant", sessionId, null)], 3)
                : new TranscriptPage([new(1, "user", "first", null), new(2, "assistant", "second", null), new(3, "assistant", sessionId, null)], null));
    }
}
