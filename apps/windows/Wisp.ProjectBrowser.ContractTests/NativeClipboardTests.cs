using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeClipboardTests
{
    public static async Task RunAsync()
    {
        foreach (var path in new[] { "C:relative.csv", "../x", "\\\\?\\C:\\x", "\\\\.\\pipe\\x", "C:\\x\n.csv", "C:/../x", "file:///C:/x", "\\\\server" })
            Check(!NativeFilePathRules.WindowsClipboardPath(path), "not a clipboard filesystem identity: " + path);
        var paths = new[] { "C:\\Research data\\图.csv", "D:/figures/", "\\\\server\\share\\image.png" };
        Check(paths.All(NativeFilePathRules.WindowsClipboardPath), "absolute Windows files, folders and UNC shares are references");
        Check(paths.All(path => !NativeFilePathRules.Relative(path)), "clipboard paths do not loosen scoped file command validation");
        var host = new Host(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenProjectDraftAsync("p"); var revision = model.FileReferenceRevision; var calls = host.Commands.Count;
        Check(model.ReferenceClipboardPaths([.. paths, paths[0].ToLowerInvariant(), "C:relative"], revision) == 3
            && model.References.Length == 3 && model.Attachments.Length == 0 && model.CanSubmit && host.Commands.Count == calls,
            "paste references files/folders without reading, copying or creating a session");
        var message = NativeComposerReferencesModel.Message("inspect", model.References);
        Check(message.Contains(JsonSerializer.Serialize(paths[0])) && message.Contains("Referenced local file paths (not imported)"),
            "absolute paths remain quoted message text");
        try { model.References[0].Reference.ToJson(); throw new Exception("file path leaked onto structured wire"); }
        catch (InvalidOperationException) { }
        model.RemoveReference(model.References[0].Reference.Key);
        Check(model.References.Length == 2 && host.Commands.Count == calls, "removing a pasted chip never deletes or writes a file");
        await model.OpenProjectDraftAsync("other");
        Check(model.ReferenceClipboardPaths(paths, revision) == 0 && model.References.Length == 0, "delayed clipboard reads cannot cross project navigation");
        await model.OpenProjectDraftAsync("p");
        Check(model.References.Length == 2, "pasted references survive returning to a draft");
        await model.OpenAsync("p", "s");
        Check(model.ReferenceClipboardPaths(paths, model.FileReferenceRevision) == 3 && model.CanSend, "existing conversation accepts clipboard paths");
        host.ReadOnly = true; await model.RefreshAsync();
        Check(model.ReferenceClipboardPaths(["C:/blocked.csv"], model.FileReferenceRevision) == 0, "read-only conversations reject pasted references");
        var reads = 0;
        Check(await model.ReferenceDroppedPathsAsync(() => { reads++; return Task.FromResult<IReadOnlyList<string>>(paths); }) == 0
            && reads == 0, "read-only drops do not request the data package");
        host.ReadOnly = false; await model.OpenProjectDraftAsync("drop"); model.Draft = "Keep this draft";
        calls = host.Commands.Count;
        Check(await model.ReferenceDroppedPathsAsync(() => Task.FromResult<IReadOnlyList<string>>([.. paths, paths[0].ToLowerInvariant(), "C:relative"])) == 3
            && model.Attachments.Length == 0 && model.CurrentSessionId == null && model.Draft == "Keep this draft" && host.Commands.Count == calls,
            "native drop deduplicates files/folders without uploads, session creation or draft replacement");
        var delayed = new TaskCompletionSource<IReadOnlyList<string>>();
        var drop = model.ReferenceDroppedPathsAsync(() => delayed.Task);
        await model.OpenProjectDraftAsync("another-project"); delayed.SetResult(["C:/late.csv"]);
        Check(await drop == 0 && model.References.Length == 0, "late data package cannot enter another project");
        var failed = new TaskCompletionSource<IReadOnlyList<string>>();
        drop = model.ReferenceDroppedPathsAsync(() => failed.Task);
        await model.OpenProjectDraftAsync("drop"); failed.SetException(new IOException("late failure"));
        Check(await drop == 0 && model.OperationError == null && model.References.Length == 3,
            "late package failure does not overwrite another draft's state");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        Check(await model.ReferenceDroppedPathsAsync(() => { reads++; return Task.FromResult<IReadOnlyList<string>>(paths); }, cancelled.Token) == 0
            && reads == 0, "cancelled drop does not read data");
        Check(await model.ReferenceDroppedPathsAsync(() => Task.FromException<IReadOnlyList<string>>(new IOException("unavailable file"))) == 0
            && model.OperationError == "unavailable file" && model.Draft == "Keep this draft" && model.References.Length == 3,
            "failed drop retains existing draft and references");
        model.Pause();
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Host : INativeSettingsClient
    {
        public bool ReadOnly;
        public List<string> Commands = [];
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Commands.Add(command);
            object? result = command switch {
                "native_conversation_snapshot" => new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", ++sequence, projectId!, args["session_id"]!.GetValue<string>(), [], null, false, false, ReadOnly, "model", null, null, [], ComposerReferences: true),
                "list_models" => new JsonArray(), _ => null };
            return Task.FromResult(result is JsonNode node ? node : JsonSerializer.SerializeToNode(result, ConversationSnapshot.JsonOptions));
        }
    }
}
