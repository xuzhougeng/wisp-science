using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeFileSelectionTests
{
    public static async Task RunAsync()
    {
        var host = new Host(); var files = new NativeFilesClient(host);
        var model = new WorkspacePanelModel(new NativePanelClient(host), "p", "s", new NativePanelTabs("[\"files\"]", "files", ["files"]), files: files);
        await model.RefreshAsync("files"); model.ToggleFileSelectionMode();
        model.ToggleFileSelection("QC.csv"); model.ToggleFileSelection("notes.md");
        Check(model.SelectedFiles.Count == 2 && model.Preview == null, "selection does not open files");
        Check(await model.CopyFilePathsAsync(false, "QC.csv") == "QC.csv\nnotes.md", "copying a selected row copies the sorted selection");
        Check(await model.CopyFilePathsAsync(true, "folder") == "C:/work/folder", "unselected clicked row copies only that path");
        Check(NativeFilePathRules.ExactUnderRoot(@"\\?\C:\work\nested\QC.csv", @"C:\work", "nested/QC.csv")
            && !NativeFilePathRules.ExactUnderRoot("C:/work-other/QC.csv", "C:/work", "QC.csv"), "Windows extended paths normalize without accepting a sibling root");
        host.Outside = true;
        Check(await model.CopyFilePathsAsync(true, "QC.csv") == null && model.Error != null, "foreign absolute path cannot reach clipboard"); host.Outside = false;
        var pending = host.PathsPending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var copying = model.CopyFilePathsAsync(false, "QC.csv");
        await model.SearchFilesAsync("QC"); pending.SetResult(host.PathReply("p", "s", ["QC.csv", "notes.md"]));
        Check(await copying == null && model.SelectedFiles.Count == 0, "new search discards a pending copy and old selection"); host.PathsPending = null;
        model.ToggleFileSelection("nested/QC.csv");
        Check(await model.CopyFilePathsAsync(false) == "nested/QC.csv", "search selection uses the result path");
        Check(model.FileDrag("nested/QC.csv")?.Path == "nested/QC.csv" && model.FileDrag("QC.csv") == null, "drag only accepts visible files");
        await model.RefreshAsync("files", "folder"); Check(model.SelectedFiles.Count == 0, "directory navigation clears selection");
        model.SelectAllFiles(); Check(model.SelectedFiles.Count == 3, "select all includes directories");
        model.SelectAllFiles(); Check(model.SelectedFiles.Count == 0, "second select all clears selection");
        await model.SelectFileLocationAsync("ssh:lab"); Check(!model.SelectingFiles && model.FileDrag("QC.csv") == null, "remote sources leave local selection and drag mode");

        var conversation = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await conversation.OpenAsync("p", "s"); var drag = new NativeFileDrag("p", "s", "nested/QC.csv");
        var before = host.PathReads;
        Check(!await conversation.ReferenceFileAsync(drag with { SessionId = "other" }) && host.PathReads == before, "cross-session drop never reads or stages a file");
        Check(await conversation.ReferenceFileAsync(drag) && await conversation.ReferenceFileAsync(drag)
            && conversation.References.Length == 1 && conversation.Attachments.Length == 0, "file path chips deduplicate without uploading");
        conversation.Draft = "Inspect"; await conversation.SendAsync();
        Check(host.Sent?["message"]?.GetValue<string>() == "Inspect\n\nReferenced local file paths (not imported): \"nested/QC.csv\""
            && (host.Sent?["references"]?.AsArray().Count ?? 0) == 0 && host.Sent?["attachments"]?.AsArray().Count == 0,
            "send persists quoted local path text without unsupported host reference kinds or uploads");
        await conversation.ReferenceFileAsync(drag); host.Running = true; await conversation.RefreshAsync(); await conversation.QueueAsync();
        Check(host.Queued?["message"]?.GetValue<string>().Contains("\"nested/QC.csv\"") == true
            && (host.Queued?["references"]?.AsArray().Count ?? 0) == 0, "queued message keeps local path references in text");
        pending = host.PathsPending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var adding = conversation.ReferenceFileAsync(drag); await conversation.OpenAsync("p", "another");
        pending.SetResult(host.PathReply("p", "s", ["nested/QC.csv"]));
        Check(!await adding && conversation.References.Length == 0, "late path resolution cannot modify another composer");
        model.Close(); conversation.Pause();
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Host : INativeSettingsClient
    {
        public bool Outside, Running;
        public int PathReads;
        public JsonObject? Sent, Queued;
        public TaskCompletionSource<JsonNode?>? PathsPending;
        private ulong sequence;
        public JsonNode? PathReply(string project, string session, string[] paths) => JsonSerializer.SerializeToNode(
            new NativeFilePaths(NativeFilesClient.SchemaId, project, session, paths.Select(p => new NativeFilePathPair(p, p, (Outside ? "C:/outside/" : "C:/work/") + p)).ToArray()), ConversationSnapshot.JsonOptions);
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            var session = args["session_id"]?.GetValue<string>() ?? "s";
            object? reply = command switch {
                "native_conversation_panel_file_locations" => new NativeFileLocations(NativeFilesClient.SchemaId, projectId!, session, "C:/work", false, [new("local", "", "local"), new("ssh:lab", "Lab", "ssh")]),
                "native_conversation_panel_file_directory" => new NativeFileDirectory(NativeFilesClient.SchemaId, projectId!, session, args["context_id"]!.GetValue<string>(), args["path"]!.GetValue<string>(),
                    [new("QC.csv", false, 18, null), new("notes.md", false, 12, null), new("folder", true, 0, null)]),
                "native_conversation_panel_searchfiles" => new NativeFileSearchHit[] { new("nested/QC.csv", "QC.csv", false, 18) },
                "native_conversation_snapshot" => new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", ++sequence, projectId!, session, [], null, Running, false, false, "model", null, null, [], ComposerReferences: true),
                "list_models" => new JsonArray(), _ => null };
            if (command == "native_conversation_panel_file_paths") { PathReads++; return PathsPending?.Task ?? Task.FromResult(PathReply(projectId!, session, args["paths"]!.AsArray().Select(p => p!.GetValue<string>()).ToArray())); }
            if (command == "native_conversation_send") { Sent = args.DeepClone().AsObject(); reply = new JsonObject(); }
            if (command == "native_conversation_enqueue") { Queued = args.DeepClone().AsObject(); reply = new JsonObject { ["queued"] = true }; }
            return Task.FromResult(reply is JsonNode node ? node : JsonSerializer.SerializeToNode(reply, ConversationSnapshot.JsonOptions));
        }
    }
}
