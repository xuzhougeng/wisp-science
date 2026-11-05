using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSearchTests
{
    public static async Task RunAsync(string fixture)
    {
        void Check(bool ok, string name) { if (!ok) throw new Exception(name); Console.WriteLine("PASS native search: " + name); }
        Check(NativePaletteActions.Match("", false, false, true).All(a => !a.Project && !a.Session), "home palette does not advertise project/session-only actions");
        Check(NativePaletteActions.Match("", true, false, true).All(a => !a.Session), "empty project does not expose unusable session panels");
        Check(NativePaletteActions.Match("no document matches", false, false, false).Select(a => a.Id)
            .SequenceEqual(new[] { "new-project", "check-updates", "star-us" }), "home search keeps its three fixed commands while typing");
        Check(NativePaletteActions.Match("", true, false, false).Select(a => a.Id)
            .SequenceEqual(new[] { "new-project", "check-updates", "star-us", "project-settings", "skills" }),
            "workspace search adds project settings and skills in the reference order");
        using var homeSearch = new WorkspaceSearchModel(() => Task.FromResult<INativeSearchClient?>(null), null);
        var restoredHost = new RestoredWorkspace();
        await homeSearch.LoadCommandProjectAsync(restoredHost, [("project-b", "C:/other"), ("project-a", "c:/work/")]);
        Check(homeSearch.CommandProjectId == "project-a" && NativePaletteActions.Match("", homeSearch.CommandProjectId != null, false, false)
            .Select(a => a.Id).SequenceEqual(["new-project", "check-updates", "star-us", "project-settings", "skills"]),
            "home settings shortcuts bind to the restored host project rather than the first project");
        using var hiddenSearch = new WorkspaceSearchModel(() => Task.FromResult<INativeSearchClient?>(null), null);
        await hiddenSearch.LoadCommandProjectAsync(restoredHost, [("project-b", "C:/other")]);
        Check(hiddenSearch.CommandProjectId == null, "hidden or foreign restored projects do not acquire settings shortcuts");
        using var ambiguousSearch = new WorkspaceSearchModel(() => Task.FromResult<INativeSearchClient?>(null), null);
        await ambiguousSearch.LoadCommandProjectAsync(restoredHost, [("project-a", "C:/work"), ("project-b", "C:/work")]);
        Check(ambiguousSearch.CommandProjectId == null, "an ambiguous workspace path never chooses a project by list order");
        restoredHost.Pending = new();
        using var closedSearch = new WorkspaceSearchModel(() => Task.FromResult<INativeSearchClient?>(null), null);
        var resolving = closedSearch.LoadCommandProjectAsync(restoredHost, [("project-a", "C:/work")]); closedSearch.Dispose();
        restoredHost.Pending.SetResult(new JsonObject { ["workspace"] = "C:/work" }); await resolving;
        Check(closedSearch.CommandProjectId == null, "a delayed restored-project lookup cannot reopen a closed palette scope");
        var p = new NativeSearchItem("project", "p", "p", "project", "project", "", null);
        var a = new NativeSearchItem("artifact", "a", "p", "project", "file", "", "s");
        var s = new NativeSearchItem("session", "s", "p", "project", "session", "", "s");
        Check(SearchPaletteLayout.Attachment(s, [p, a, s]) == s && SearchPaletteLayout.Attachment(p, [p, a, s]) == a
            && SearchPaletteLayout.Attachment(null, [p]) == null, "Shift+Enter uses the selected reference or the first attachable match");
        Check(NativePaletteActions.Match("files", true, true, true).Single().Id == "files", "English keyword finds the native file action");
        var commands = NativePaletteActions.Match("", true, true, true).ToArray();
        Check(commands.Select(a => a.Icon).Distinct().Count() == commands.Length, "actions have distinct shared icons");
        var root = Path.GetFullPath(Path.Combine(Path.GetDirectoryName(fixture)!, "../../.."));
        var catalog = System.Text.Json.JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(
            File.ReadAllText(Path.Combine(root, "apps/macos/Sources/WispProjectBrowserUI/Resources/native-locales.json")))!;
        var locale = new NativeLocalization(catalog);
        foreach (var code in new[] { "en", "zh" })
        {
            locale.Preview(code);
            var transfer = NativePaletteActions.Match(locale.Text("command.group.transfer"), true, false, true, locale).Select(x => x.Id);
            Check(transfer.SequenceEqual(new[] { "import-codex", "import-claude", "import-session", "export-current-project" })
                && !NativePaletteActions.Match(locale.Text("command.group.transfer"), false, false, true, locale).Any(),
                $"{code} group search lists all four transfers in order only inside a project, even without a session");
            Check(NativePaletteActions.Match("github", false, false, true, locale).Select(x => x.Id).SequenceEqual(new[] { "star-us", "issues" })
                && NativePaletteActions.Match("upgrade", false, false, true, locale).Single().Id == "check-updates",
                "help links and update checking are available without opening a project");
            Check(NativePaletteActions.Match("font-code-decrease", false, false, true, locale).Single().Id == "font-code-decrease"
                && NativePaletteActions.Match(locale.Text("command.group.appearance"), false, false, true, locale).Count() == 7,
                "the full appearance group can be found by ID or localized section name");
            foreach (var action in commands)
            {
                var label = action.Label(locale);
                Check(!string.IsNullOrWhiteSpace(label) && !label.StartsWith("command.") && !label.StartsWith("menu.")
                    && NativePaletteActions.Match(label, true, true, true, locale).Any(a => a.Id == action.Id),
                    $"{code} palette finds {action.Id} by its visible label");
            }
            Check(commands.Single(a => a.Id == "new-project").Label(locale) == (code == "en" ? "New project" : "新建项目"),
                "palette follows the shared project creation label");
        }
        Check(SearchPaletteLayout.StepChoice(0, 3, false) == 2 && SearchPaletteLayout.StepChoice(2, 3, true) == 0
            && SearchPaletteLayout.StepChoice(-1, 1, false) == 0 && SearchPaletteLayout.StepChoice(-1, 0, true) == -1,
            "search wraps between actionable rows and handles an empty or one-result list");
        Check(!SearchPaletteLayout.ListOverflows(0) && !SearchPaletteLayout.ListOverflows(1) && SearchPaletteLayout.ListOverflows(2),
            "fitting lists have no footer fade, including fractional layout rounding");
        Check(commands.All(a => File.Exists(Path.Combine(root, "apps/macos/Sources/WispProjectBrowserUI/Resources", "icon-" + a.Icon + ".svg"))),
            "every palette icon is shipped so opening the palette cannot fail on a missing resource");
        var json = JsonNode.Parse(File.ReadAllText(fixture))!;
        var decoded = NativeSearchClient.Decode(json["result"], "needle", "research-1");
        Check(decoded.Items.Single().SessionId == "session-1", "shared fixture retains the exact owner and session");
        foreach (var mutate in new Action<JsonNode>[] {
            n => n["query"] = "other", n => n["preferred_project_id"] = "foreign",
            n => n["items"]![0]!["session_id"] = "foreign", n => n["items"]![0]!["kind"] = "unknown",
            n => n["items"]!.AsArray().Add(n["items"]![0]!.DeepClone()) })
        {
            var bad = json["result"]!.DeepClone(); mutate(bad);
            try { NativeSearchClient.Decode(bad, "needle", "research-1"); throw new Exception("accepted invalid search response"); }
            catch (InvalidDataException) { }
        }
        var client = new Fake();
        var model = new WorkspaceSearchModel(() => Task.FromResult<INativeSearchClient?>(client), "research-1");
        var delayed = new TaskCompletionSource<NativeSearchResponse>(); client.Pending = delayed;
        var first = model.SearchAsync("old");
        client.Pending = null; await model.SearchAsync("new");
        delayed.SetResult(Fake.Response("old", "research-1")); await first;
        Check(model.Items.Single().Title == "new" && !model.Busy, "late older query cannot replace the new results");
        client.Pending = delayed = new();
        var closing = model.SearchAsync("closing"); model.Dispose();
        delayed.SetResult(Fake.Response("closing", "research-1")); await closing;
        Check(model.Items.Length == 0, "Escape disposal rejects a delayed response even when transport ignores cancellation");
        var heldConnect = new TaskCompletionSource<INativeSearchClient?>();
        var lateModel = new WorkspaceSearchModel(() => heldConnect.Task, null);
        var late = lateModel.SearchAsync("late"); lateModel.Dispose(); var calls = client.Calls;
        heldConnect.SetResult(client); await late;
        Check(client.Calls == calls, "closing while connecting never dispatches a stale read");
        client.Pending = null; client.Fail = true;
        using var failing = new WorkspaceSearchModel(() => Task.FromResult<INativeSearchClient?>(client), null, locale);
        await failing.SearchAsync("private"); calls = client.Calls;
        Check(failing.Error != null && failing.Items.Length == 0 && !failing.Busy, "failed search displays an error without exposing cached results");
        await failing.SearchAsync(new string('中', 172));
        Check(client.Calls == calls && failing.Error?.Contains("过长") == true, "UTF-8 query ceiling is reported in Chinese before transport");
        locale.Preview("en"); await failing.SearchAsync(new string('中', 172));
        Check(client.Calls == calls && failing.Error?.Contains("too long") == true, "validation follows the current English locale");
        client.Fail = false;
        using var debounced = new WorkspaceSearchModel(() => Task.FromResult<INativeSearchClient?>(client), null);
        var cancelled = debounced.SearchAsync("cancelled", TimeSpan.FromSeconds(10));
        await debounced.SearchAsync("current"); await cancelled;
        Check(client.Calls == calls + 1 && debounced.Items.Single().Title == "current", "superseded debounce never reaches the host");
    }
    private sealed class RestoredWorkspace : INativeSettingsClient
    {
        public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command != "get_bootstrap_status" || projectId != null || arguments.Count != 0) throw new InvalidOperationException("Unexpected workspace lookup");
            return Pending?.Task ?? Task.FromResult<JsonNode?>(new JsonObject { ["workspace"] = "C:\\work" });
        }
    }
    private sealed class Fake : INativeSearchClient
    {
        public TaskCompletionSource<NativeSearchResponse>? Pending;
        public bool Fail;
        public int Calls;
        public static NativeSearchResponse Response(string query, string? project) => new(NativeSearchClient.Schema, query, project,
            [new("session", "s", project ?? "p", "Research", query, "Research", "s")]);
        public Task<NativeSearchResponse> SearchAsync(string query, string? preferredProject, CancellationToken cancellationToken = default)
        {
            Calls++;
            if (Fail) throw new IOException("privacy unavailable");
            return Pending?.Task ?? Task.FromResult(Response(query, preferredProject));
        }
    }
}
