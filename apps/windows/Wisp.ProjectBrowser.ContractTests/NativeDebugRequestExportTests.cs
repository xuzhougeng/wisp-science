using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeDebugRequestExportTests
{
    public static async Task RunAsync(string fixtureDirectory)
    {
        var fixture = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtureDirectory, "../../native-session-export/v1/debug-request.json")))!;
        var file = fixture["result"]!.Deserialize<NativeDebugRequestFile>(ConversationSnapshot.JsonOptions)!;
        var source = new BrowserSession(file.SessionId, file.ProjectId, "Synthetic", 1, "complete");
        var path = Path.Combine(Path.GetTempPath(), "synthetic-debug.json");
        var fake = new Fake();
        int pickers = 0, writes = 0;
        Task<string?> Pick(string name) { Check(name == file.Filename, "shared filename"); pickers++; return Task.FromResult<string?>(path); }
        Task Write(string destination, byte[] contents)
        {
            writes++;
            Check(destination == path && contents.SequenceEqual(Encoding.UTF8.GetBytes(file.Contents)), "exact captured UTF-8 bytes, without reserialization or BOM");
            return Task.CompletedTask;
        }
        using (var export = new WorkspaceDebugRequestExport(fake, source))
        {
            var pending = export.ExportAsync(Pick, Write, () => true);
            Check(fake.Calls[^1].Command == "native_conversation_debug_request" && fake.Calls[^1].Project == source.ProjectId
                && fake.Calls[^1].Args["session_id"]!.ToString() == source.Id, "explicit owner and session read");
            Check(!await export.ExportAsync(Pick, Write, () => true), "duplicate export suppressed");
            fake.Reply(file); await pending;
            Check(export.SavedPath == path && writes == 1 && pickers == 1, "one successful write");
        }
        foreach (var bad in new[] { file with { ProjectId = "other" }, file with { SessionId = "other" },
            file with { Schema = "future" }, file with { Filename = "../escape.json" },
            file with { Contents = file.Contents.Replace(source.Id, "other") },
            file with { Contents = "{\"session_id\":\"source-1\",\"source\":\"stored-messages\",\"message_count\":0,\"messages\":[]}" } })
        {
            using var export = new WorkspaceDebugRequestExport(fake, source);
            var pending = export.ExportAsync(Pick, Write, () => true); fake.Reply(bad); await pending;
            Check(export.Error != null && export.SavedPath == null && pickers == 1 && writes == 1, "invalid payload rejected before picker");
        }
        using (var cancelled = new WorkspaceDebugRequestExport(fake, source))
        {
            var pending = cancelled.ExportAsync(_ => Task.FromResult<string?>(null), Write, () => true);
            fake.Reply(file); await pending;
            Check(cancelled.Error == null && cancelled.SavedPath == null && writes == 1, "picker cancellation never writes");
        }
        using (var stale = new WorkspaceDebugRequestExport(fake, source))
        {
            var picker = new TaskCompletionSource<string?>(); var current = true;
            var pending = stale.ExportAsync(_ => picker.Task, Write, () => current);
            fake.Reply(file); current = false; picker.SetResult(path); await pending;
            Check(stale.SavedPath == null && writes == 1, "navigation during picker suppresses write");
        }
        using (var closed = new WorkspaceDebugRequestExport(fake, source))
        {
            var pending = closed.ExportAsync(Pick, Write, () => true); closed.Dispose(); fake.Reply(file); await pending;
            Check(closed.SavedPath == null && pickers == 1 && writes == 1, "close before capture suppresses picker");
        }
        using (var unknown = new WorkspaceDebugRequestExport(fake, source))
        {
            var pending = unknown.ExportAsync(Pick, (_, _) => throw new IOException("Synthetic interrupted write"), () => true);
            fake.Reply(file); await pending;
            var calls = fake.Calls.Count;
            Check(unknown.WriteUnconfirmed && unknown.Error != null && !await unknown.ExportAsync(Pick, Write, () => true)
                && fake.Calls.Count == calls, "failed write cannot automatically replay");
        }
        var live = file with { Contents = file.Contents.Replace("stored-messages", "live-agent") };
        using var liveExport = new WorkspaceDebugRequestExport(fake, source);
        var livePending = liveExport.ExportAsync(_ => Task.FromResult<string?>(null), Write, () => true);
        fake.Reply(live); await livePending; Check(liveExport.Error == null, "both shared snapshot sources accepted");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project, TaskCompletionSource<JsonNode?> Reply)> Calls { get; } = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        { var reply = new TaskCompletionSource<JsonNode?>(); Calls.Add((command, args, projectId, reply)); return reply.Task; }
        public void Reply(NativeDebugRequestFile file) => Calls[^1].Reply.SetResult(JsonSerializer.SerializeToNode(file, ConversationSnapshot.JsonOptions));
    }
}
