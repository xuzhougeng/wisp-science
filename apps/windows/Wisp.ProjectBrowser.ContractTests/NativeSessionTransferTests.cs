using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSessionTransferTests
{
    public static async Task RunAsync(string fixtureDirectory)
    {
        var transferFixture = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtureDirectory, "../../native-session-transfer/v1/move.json")))!;
        var transferPreview = transferFixture["preview"]!.Deserialize<NativeSessionTransferPreview>(ConversationSnapshot.JsonOptions)!;
        var transferResult = transferFixture["result"]!.Deserialize<NativeSessionTransferResult>(ConversationSnapshot.JsonOptions)!;
        var source = new BrowserSession("source-1", "research-1", "RNA-seq comparison", 1, "complete");
        var fake = new Fake();
        ProjectSummary Project(string id) => new(id, id, "", "", false, 0, 0, 1, 0, 0, false, null);
        var targets = new[] { Project(source.ProjectId), Project("research-2"), Project("third"), Project("assistant:global") };
        using var transfer = new WorkspaceSessionTransfer(fake, source, "move", targets);
        Check(transfer.Targets.Select(p => p.Id).SequenceEqual(["research-2", "third"]), "source and assistant cannot be targets");
        var oldRead = transfer.LoadAsync(); var newRead = transfer.ChooseTargetAsync("third");
        fake.Calls[1].Reply.SetResult(Node(transferPreview with { TargetProjectId = "third" })); await newRead;
        fake.Calls[0].Reply.SetResult(Node(transferPreview)); await oldRead;
        Check(transfer.Preview!.TargetProjectId == "third" && transfer.CanTransfer, "late target preview is ignored");
        transfer.ChooseArtifacts(true); var write = transfer.TransferAsync();
        Check(!await transfer.TransferAsync(), "duplicate transfer is suppressed");
        Check(fake.Calls[^1].Project == source.ProjectId && fake.Calls[^1].Args["artifact_fingerprint"]!.ToString() == transferPreview.Artifacts!.Fingerprint, "reviewed file plan and explicit source are sent");
        fake.Calls[^1].Reply.SetResult(Node(transferResult with { TargetProjectId = "third" }));
        Check(await write && !transfer.CanTransfer, "matching result completes once");
        using var uncertain = new WorkspaceSessionTransfer(fake, source, "copy", targets);
        var read = uncertain.LoadAsync(); fake.Calls[^1].Reply.SetResult(Node(transferPreview with { Mode = "copy", Artifacts = null })); await read;
        uncertain.ChooseArtifacts(true); Check(!uncertain.IncludeArtifacts, "copies never move artifacts");
        var unknown = uncertain.TransferAsync(); fake.Calls[^1].Reply.SetResult(Node(transferResult));
        Check(!await unknown && uncertain.MutationUnconfirmed && !uncertain.CanTransfer, "wrong-mode acknowledgement blocks replay");
        var count = fake.Calls.Count; await uncertain.LoadAsync(); await uncertain.ChooseTargetAsync("third"); await uncertain.TransferAsync();
        Check(fake.Calls.Count == count, "uncertain transfer cannot be refreshed into an accidental retry");
        using var artifactFailure = new WorkspaceSessionTransfer(fake, source, "move", targets);
        read = artifactFailure.LoadAsync(); fake.Calls[^1].Reply.SetResult(Node(transferPreview with { Artifacts = null, ArtifactError = "Unavailable" })); await read;
        artifactFailure.ChooseArtifacts(true); Check(!artifactFailure.CanTransfer, "artifact error blocks selected files");
        artifactFailure.ChooseArtifacts(false); Check(artifactFailure.CanTransfer, "transcript-only transfer remains possible");
        using var closed = new WorkspaceSessionTransfer(fake, source, "move", targets);
        read = closed.LoadAsync(); closed.Dispose(); fake.Calls[^1].Reply.SetResult(Node(transferPreview)); await read;
        Check(closed.Preview == null && !closed.CanTransfer, "close invalidates pending preview");
        using var empty = new WorkspaceSessionTransfer(fake, source, "copy", [Project(source.ProjectId)]);
        count = fake.Calls.Count; await empty.LoadAsync(); Check(fake.Calls.Count == count && !empty.CanTransfer, "no target does not issue a request");

        var exportFixture = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtureDirectory, "../../native-session-export/v1/export.json")))!;
        var exportPreview = exportFixture["preview"]!.Deserialize<NativeSessionExportPreview>(ConversationSnapshot.JsonOptions)!;
        var exportResult = exportFixture["result"]!.Deserialize<NativeSessionExportResult>(ConversationSnapshot.JsonOptions)!;
        var exportSource = source with { Id = exportPreview.SessionId, ProjectId = exportPreview.ProjectId };
        var path = Path.Combine(Path.GetTempPath(), "synthetic-session.zip");
        using var cancelled = new WorkspaceSessionExport(fake, exportSource);
        var export = cancelled.ExportAsync(_ => Task.FromResult<string?>(null), () => true);
        fake.Calls[^1].Reply.SetResult(Node(exportPreview)); await export;
        Check(cancelled.Result == null && cancelled.Error == null && !cancelled.Busy, "cancelled picker does not write");
        using var stale = new WorkspaceSessionExport(fake, exportSource);
        var picker = new TaskCompletionSource<string?>(); var current = true;
        export = stale.ExportAsync(_ => picker.Task, () => current);
        fake.Calls[^1].Reply.SetResult(Node(exportPreview)); count = fake.Calls.Count; current = false; picker.SetResult(path); await export;
        Check(fake.Calls.Count == count && stale.Result == null, "navigation during picker suppresses write");
        using var saving = new WorkspaceSessionExport(fake, exportSource);
        export = saving.ExportAsync(_ => Task.FromResult<string?>(path), () => true);
        fake.Calls[^1].Reply.SetResult(Node(exportPreview));
        Check(!await saving.ExportAsync(_ => throw new Exception("must not open second picker"), () => true), "one export in flight");
        // TCS continuations run synchronously, so the acknowledged preview has
        // already started the write to our fake, without touching the filesystem.
        Check(fake.Calls[^1].Command == "native_conversation_export" && fake.Calls[^1].Args["revision"]!.ToString() == exportPreview.Revision, "save binds reviewed revision");
        fake.Calls[^1].Reply.SetResult(Node(exportResult with { DestinationPath = path })); await export;
        Check(saving.Result?.DestinationPath == path, "matching archive result is accepted");
        using var badSave = new WorkspaceSessionExport(fake, exportSource);
        export = badSave.ExportAsync(_ => Task.FromResult<string?>(path), () => true);
        fake.Calls[^1].Reply.SetResult(Node(exportPreview)); fake.Calls[^1].Reply.SetException(new IOException("Lost acknowledgement")); await export;
        count = fake.Calls.Count; await badSave.ExportAsync(_ => Task.FromResult<string?>(path), () => true);
        Check(badSave.MutationUnconfirmed && fake.Calls.Count == count, "unknown archive write cannot replay");
    }
    private static JsonNode Node<T>(T value) => JsonSerializer.SerializeToNode(value, ConversationSnapshot.JsonOptions)!;
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project, TaskCompletionSource<JsonNode?> Reply)> Calls { get; } = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        { var reply = new TaskCompletionSource<JsonNode?>(); Calls.Add((command, args, projectId, reply)); return reply.Task; }
    }
}
