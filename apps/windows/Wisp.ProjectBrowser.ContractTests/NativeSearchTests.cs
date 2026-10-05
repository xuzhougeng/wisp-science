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
        Check(NativePaletteActions.Match("files", true, true, true).Single().Id == "files", "English keyword finds the native file action");
        var commands = NativePaletteActions.Match("", true, true, true).ToArray();
        Check(commands.Select(a => a.Icon).Distinct().Count() == commands.Length, "actions have distinct shared icons");
        var root = Path.GetFullPath(Path.Combine(Path.GetDirectoryName(fixture)!, "../../.."));
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
        using var failing = new WorkspaceSearchModel(() => Task.FromResult<INativeSearchClient?>(client), null);
        await failing.SearchAsync("private"); calls = client.Calls;
        Check(failing.Error != null && failing.Items.Length == 0 && !failing.Busy, "failed search displays an error without exposing cached results");
        await failing.SearchAsync(new string('中', 172));
        Check(client.Calls == calls && failing.Error != null, "UTF-8 query ceiling is enforced before transport");
        client.Fail = false;
        using var debounced = new WorkspaceSearchModel(() => Task.FromResult<INativeSearchClient?>(client), null);
        var cancelled = debounced.SearchAsync("cancelled", TimeSpan.FromSeconds(10));
        await debounced.SearchAsync("current"); await cancelled;
        Check(client.Calls == calls + 1 && debounced.Items.Single().Title == "current", "superseded debounce never reaches the host");
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
