using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeShelvedSessionsTests
{
    public static async Task RunAsync(string fixtureDirectory)
    {
        var fixture = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtureDirectory, "../../native-projects/v1/shelved.json")))!;
        var page = fixture["page"]!.Deserialize<ShelvedSessionPage>()!;
        Check(page.ProjectId == "research-1" && page.Items.Single().Title == "RNA 研究" && page.NextCursor?.Id == "session-a", "shared page fixture");
        Check(fixture["restore"]!.Deserialize<SessionShelveResult>() is { Shelved: false, SessionId: "session-a" }, "shared restore fixture");
        var client = new Fake(); using var model = new WorkspaceShelvedSessions(client, "p");
        var old = model.SearchAsync("old"); var latest = model.SearchAsync("new");
        client.Reply(1, new("p", [new("b", "New", 20)], new(20, "b"))); await latest;
        client.Reply(0, new("p", [new("a", "Old", 10)], null)); await old;
        Check(model.Query == "new" && model.Items.Single().Id == "b", "late search cannot overwrite current query");
        var more = model.MoreAsync(); await model.MoreAsync();
        Check(client.Reads.Count == 3 && client.Args[2]["cursor"]!["id"]!.ToString() == "b", "keyset and duplicate paging guard");
        client.Reply(2, new("p", [new("b", "New", 20), new("c", "Older", 15)], null)); await more;
        Check(model.Items.Length == 2, "deduplicate page boundaries");
        var restore = model.RestoreAsync("b"); Check(!await model.RestoreAsync("b"), "duplicate restore suppressed");
        client.Write!.SetResult(JsonSerializer.SerializeToNode(new SessionShelveResult("p", "b", false)));
        Check(await restore && model.DidRestore && model.Items.Single().Id == "c", "acknowledged restore removes only its own row");
        var failed = model.RestoreAsync("c"); client.Write!.SetException(new IOException("Lost acknowledgement"));
        Check(!await failed && model.Items.Single().Id == "c" && model.RestoreUnconfirmed, "unknown restore preserves row and exposes ambiguity");
        Check(!await model.RestoreAsync("c") && client.Writes == 2, "ambiguous mutation is not replayed");
        var pending = model.SearchAsync("closed"); model.Dispose(); client.Reply(3, new("p", [], null)); await pending;
        Check(model.Items.Length == 0, "closing ignores late data");
        using var scoped = new WorkspaceShelvedSessions(client, "p"); var crossed = scoped.SearchAsync("");
        client.Reply(4, new("other", [new("bad", "Wrong project", 1)], null)); await crossed;
        Check(scoped.Error != null && scoped.Items.Length == 0 && client.Projects.All(p => p == "p"), "explicit scope and cross-project reply rejection");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public List<TaskCompletionSource<JsonNode?>> Reads { get; } = [];
        public List<JsonObject> Args { get; } = [];
        public List<string?> Projects { get; } = [];
        public TaskCompletionSource<JsonNode?>? Write; public int Writes;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Projects.Add(projectId);
            var reply = new TaskCompletionSource<JsonNode?>(TaskCreationOptions.RunContinuationsAsynchronously);
            if (command == "native_project_shelved_sessions") { Reads.Add(reply); Args.Add(args); }
            else if (command == "native_project_session_shelve") { Writes++; Write = reply; }
            else throw new InvalidOperationException(command);
            return reply.Task;
        }
        public void Reply(int index, ShelvedSessionPage page) => Reads[index].SetResult(JsonSerializer.SerializeToNode(page));
    }
}
