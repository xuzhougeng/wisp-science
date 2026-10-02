using System.Text.Json.Nodes;
using System.Text.Json;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeRunReviewTests
{
    public static async Task RunAsync(string projectFixture)
    {
        var path = System.IO.Path.GetFullPath(System.IO.Path.Combine(System.IO.Path.GetDirectoryName(projectFixture)!, "../../native-conversations/v1/run-review.json"));
        var fixture = JsonNode.Parse(File.ReadAllText(path))!;
        Check(fixture["activity"]!.Deserialize<NativeContextActivity>(ConversationSnapshot.JsonOptions)!.RunReviewSupported == true,
            "new hosts explicitly advertise review support");
        Check(JsonSerializer.Deserialize<NativeContextActivity>("""{"runtimes":[],"runs":[],"read_only":false}""", ConversationSnapshot.JsonOptions)!.RunReviewSupported == null,
            "old hosts omit review capability rather than displaying a dead action");
        var transport = new Fake { Reply = (JsonObject)fixture["reply"]!.DeepClone() };
        var model = new WorkspaceRunReviewModel(new NativeRunReviewClient(transport), "project-a", "session-a");
        await model.OpenAsync("run-a");
        Check(model.Ready && model.Entries.Length == 2 && model.Truncated, "shared listing fixture decodes");
        Check(transport.Calls.Single().Project == "project-a" && transport.Calls[0].Args["session_id"]!.GetValue<string>() == "session-a", "review explicitly scopes its session");
        model.Select(model.Entries[0], true); model.Select(model.Entries[1], true);
        Check(model.BeginConfirmation("delete") && model.ConfirmedPaths.Length == 2, "delete freezes the exact file/directory selection");
        model.HandleEscape();
        Check(model.Visible && model.Confirmation == null && model.Selection.Count == 2 && transport.Calls.Count == 1, "first Escape cancels confirmation only, retaining parent and selection without mutation");
        model.HandleEscape(); Check(!model.Visible && transport.Calls.Count == 1, "second Escape closes review without a write");
        await model.OpenAsync("run-a");
        model.Select(model.Entries[0], true); model.Select(model.Entries[1], true);
        transport.Handler = args =>
        {
            var reply = (JsonObject)transport.Reply.DeepClone();
            if (args["operation"]!["action"]!.GetValue<string>() == "download") reply["downloaded"] = 2;
            return Task.FromResult<JsonNode?>(reply);
        };
        await model.DownloadAsync();
        var download = transport.Calls.Single(call => call.Args["operation"]!["action"]!.GetValue<string>() == "download").Args["operation"]!;
        Check(download["files"]![0]!.GetValue<string>() == "results/table.tsv" && download["dirs"]![0]!.GetValue<string>() == "results/figures", "download preserves file/directory selection kinds");
        Check(model.Selection.Count == 0 && model.Status?.Contains("2") == true && !model.Mutating, "confirmed download clears selection and retains acknowledgement through refresh");

        transport.Handler = null; model.Select(model.Entries[0], true);
        model.BeginConfirmation("delete"); await model.ConfirmAsync();
        Check(transport.Calls.Any(call => call.Args["operation"]!["action"]!.GetValue<string>() == "delete"
            && call.Args["operation"]!["confirmed"]!.GetValue<bool>()), "delete sends an explicit confirmation only after confirm");
        model.Select(model.Entries[0], true);
        var before = transport.Calls.Count;
        transport.Handler = args => args["operation"]!["action"]!.GetValue<string>() == "download"
            ? throw new IOException("lost reply") : Task.FromResult<JsonNode?>(transport.Reply.DeepClone());
        await model.DownloadAsync(); await model.DownloadAsync();
        Check(transport.Calls.Count == before + 1 && model.Selection.Count == 1 && !model.CanChange && model.Error?.Contains("未确认") == true, "lost mutation response preserves selection and blocks replay until a fresh read");
        await model.ReadAsync(); Check(model.CanChange, "explicit read restores controls without repeating mutation");

        var hold = new TaskCompletionSource<JsonNode?>();
        transport.Handler = _ => hold.Task;
        var read = model.NavigateAsync("old"); model.Close();
        hold.SetResult(transport.Reply.DeepClone()); await read;
        Check(!model.Visible && !model.Ready && !model.Loading, "closed review rejects late reads");
        transport.Handler = null; transport.Reply["read_only"] = true;
        await model.OpenAsync("run-a"); model.Select(model.Entries[0], true);
        before = transport.Calls.Count;
        await model.DownloadAsync(); Check(!model.BeginConfirmation("cleanup") && transport.Calls.Count == before, "inherited/read-only review never mutates");
        transport.Reply["read_only"] = false; await model.ReadAsync();
        transport.Handler = args => Task.FromResult<JsonNode?>(new JsonObject { ["run_id"] = "other", ["acknowledged"] = true });
        await model.ReadAsync(); Check(!model.Ready && model.Error != null, "foreign run responses are rejected");
        transport.Handler = null; await model.ReadAsync(); model.BeginConfirmation("cleanup");
        transport.Handler = args =>
        {
            var reply = (JsonObject)transport.Reply.DeepClone();
            reply["cleaned"] = true; reply["listing"] = new JsonObject { ["entries"] = new JsonArray(), ["truncated"] = false };
            return Task.FromResult<JsonNode?>(reply);
        };
        await model.ConfirmAsync(); Check(model.Cleaned && !model.CanChange && model.Entries.Length == 0, "confirmed cleanup shows the cleaned state without offering another write");

        var pagingTransport = new Fake { Reply = (JsonObject)fixture["reply"]!.DeepClone() };
        var paging = new WorkspaceRunReviewModel(new NativeRunReviewClient(pagingTransport), "project-a", "session-a");
        await paging.OpenAsync("run-a"); paging.Select(paging.Entries[0], true);
        pagingTransport.Handler = args =>
        {
            Check(args["operation"]!["offset"]!.GetValue<int>() == 2, "next page uses consumed server rows");
            var reply = (JsonObject)pagingTransport.Reply.DeepClone();
            reply["listing"] = JsonNode.Parse("""{"entries":[{"path":"tail.txt","kind":"file","size_bytes":2}],"truncated":false}""");
            return Task.FromResult<JsonNode?>(reply);
        };
        await paging.ReadAsync(true);
        Check(paging.Entries.Length == 3 && !paging.Truncated && paging.Selection.Count == 1, "pagination appends without losing selected paths");
        var oldDirectory = new TaskCompletionSource<JsonNode?>();
        pagingTransport.Handler = args => args["operation"]!["path"]!.GetValue<string>() == "old" ? oldDirectory.Task : Task.FromResult<JsonNode?>(pagingTransport.Reply.DeepClone());
        var oldRead = paging.NavigateAsync("old"); await paging.NavigateAsync("new");
        oldDirectory.SetException(new IOException("stale directory")); await oldRead;
        Check(paging.Path == "new" && paging.Ready && paging.Error == null && !paging.Loading, "late directory failure cannot replace the current directory");
        pagingTransport.Handler = null; paging.BeginConfirmation("cleanup");
        await paging.ConfirmAsync();
        Check(!paging.Cleaned && !paging.Ready && paging.Error?.Contains("未确认") == true, "cleanup without authoritative cleaned acknowledgement remains uncertain");
        await paging.ReadAsync(); paging.Select(paging.Entries[0], true);
        var pendingWrite = new TaskCompletionSource<JsonNode?>();
        pagingTransport.Handler = _ => pendingWrite.Task;
        var pendingDownload = paging.DownloadAsync(); paging.Close();
        await paging.OpenAsync("new-run");
        var downloadReply = (JsonObject)pagingTransport.Reply.DeepClone(); downloadReply["downloaded"] = 1;
        pendingWrite.SetResult(downloadReply); await pendingDownload;
        Check(paging.RunId == "new-run" && paging.Visible && !paging.Ready && !paging.Mutating && paging.Status == null, "late write completion cannot mark a reopened different run as downloaded");
        Console.WriteLine("Native Run review scope, confirmation, selection, uncertainty, readonly and lifecycle checks passed.");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public required JsonObject Reply;
        public Func<JsonObject, Task<JsonNode?>>? Handler;
        public List<(string? Project, JsonObject Args)> Calls { get; } = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command != "native_conversation_panel_run_review") throw new InvalidOperationException(command);
            Calls.Add((projectId, (JsonObject)arguments.DeepClone()));
            return Handler?.Invoke(arguments) ?? Task.FromResult<JsonNode?>(Reply.DeepClone());
        }
    }
}
