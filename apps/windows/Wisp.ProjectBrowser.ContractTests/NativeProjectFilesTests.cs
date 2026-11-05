using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProjectFilesTests
{
    public static async Task RunAsync()
    {
        var host = new Host(); var files = new NativeFilesClient(host);
        var panel = new WorkspacePanelModel(new NativePanelClient(host), "p", null,
            new NativePanelTabs("[\"files\"]", "files", ["files", "memos"]), files: files);
        await panel.RefreshAsync("files");
        Check(panel.Files.Single().Name == "sample.csv" && panel.CanMutateLocalFiles && host.Creates == 0, "project browsing is writable without creating a frame");
        panel.ToggleFileSelectionMode(); panel.SelectAllFiles();
        Check(await panel.CopyFilePathsAsync(false) == "sample.csv", "project path copying keeps null session identity");
        await panel.SearchFilesAsync("sample");
        Check(panel.FileSearchHits.Single().Path == "nested/sample.csv", "project search routes without a frame");
        await panel.ReadFileAsync("nested/sample.csv");
        Check(panel.Preview?.Text == "a,b" && panel.CanEditPreview, "project files support previews and editing");
        await panel.SaveFileAsync("a,b", "a,c");
        Check(host.Commands.Contains("native_project_file_save"), "project save routes to the project host operation");
        var calls = host.Commands.Count; await panel.RefreshAsync("hosts");
        Check(host.Commands.Count == calls && panel.Tabs.Selected == "files", "sessionless file panel cannot invoke conversation tabs");
        var draft = new WorkspacePanelModel(new NativePanelClient(host), "p", null, new NativePanelTabs(available: ["files", "hosts"]));
        calls = host.Commands.Count; await draft.RefreshAsync("hosts");
        Check(draft.Tabs.Selected == "hosts" && draft.Contexts?.Attached.Single().Id == "local" && draft.HostAliases.SequenceEqual(["cluster"])
            && draft.CanManageHosts && !draft.CanChangeContext && host.Commands.Skip(calls).SequenceEqual(["native_runtimes_read", "list_ssh_hosts"]) && host.Creates == 0,
            "opening Environment on a draft reads the project registry without creating a frame or enabling session writes");
        host.ForeignSession = true;
        try { await new NativePanelClient(host).ProjectContextsAsync("p"); throw new InvalidOperationException("draft accepted a session environment snapshot"); }
        catch (InvalidDataException) { }
        try { await files.LocationsAsync("p", null); throw new InvalidOperationException("foreign frame accepted"); }
        catch (InvalidDataException) { }
        host.ForeignSession = false;

        var conversation = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await conversation.OpenProjectDraftAsync("p");
        Check(conversation.Options.IsDraft && conversation.Options.Session == null && host.Creates == 0,
            "opening a project binds option catalogs to the draft without fabricating a session");
        var drag = new NativeFileDrag("p", null, "sample.csv");
        Check(await conversation.ReferenceFileAsync(drag) && conversation.CanSubmit && host.Creates == 0, "a path alone can be staged in a project draft");
        Check(!await conversation.ReferenceFileAsync(drag with { SessionId = "foreign" }), "frame paths cannot cross into a project draft");
        await conversation.OpenProjectDraftAsync("other"); await conversation.OpenProjectDraftAsync("p");
        Check(conversation.References.Length == 1, "draft paths survive project navigation");
        conversation.RemoveReference(conversation.References.Single().Reference.Key);
        Check(conversation.References.Length == 0 && !conversation.CanSubmit, "draft path chip can be removed");
        await conversation.ReferenceFileAsync(drag);
        Check(await conversation.EnsureDraftSessionAsync() == "created" && host.Creates == 1
            && conversation.References.Single().Reference.Id == "sample.csv", "explicit send preparation carries paths into the confirmed frame");
        Check(conversation.Snapshot?.PlanMode == true, "confirmed frame exposes its enabled plan mode");
        await conversation.OpenProjectDraftAsync("p");
        Check(conversation.CurrentSessionId == null && conversation.Snapshot == null && conversation.Options.IsDraft
            && conversation.Options.Session == null && host.Creates == 1,
            "returning from an enabled session to a draft clears session options without creating another frame");
        panel.Tabs.Show("memos"); await panel.RefreshAsync("memos");
        await panel.RefreshAsync("files"); await panel.ReadFileAsync("sample.csv"); panel.EditPreview();
        if (!panel.SelectingFiles) panel.ToggleFileSelectionMode();
        panel.SelectAllFiles();
        panel.Tabs.Remove("files");
        Check(panel.Tabs.Selected == "memos", "closing selected file tab selects its left neighbor");
        calls = host.Commands.Count; await panel.RefreshAsync(panel.Tabs.Selected);
        Check(panel.Preview == null && !panel.PreviewEditable && !panel.SelectingFiles && host.Commands.Count == calls,
            "tab closure clears the previous file preview and selection even after tab state already selected its neighbor");
        panel.Close(); conversation.Pause();
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Host : INativeSettingsClient
    {
        public readonly List<string> Commands = [];
        public int Creates;
        public bool ForeignSession;
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Commands.Add(command);
            if (command.StartsWith("native_project_file_") && args["session_id"] != null) throw new InvalidOperationException("Project command leaked a frame");
            var session = ForeignSession ? "foreign" : null;
            object? result = command switch {
                "native_project_file_locations" => new NativeFileLocations(NativeFilesClient.SchemaId, projectId!, session, "C:/work", false, [new("local", "", "local")]),
                "native_project_file_directory" => new NativeFileDirectory(NativeFilesClient.SchemaId, projectId!, session, "local", ".", [new("sample.csv", false, 3, null)]),
                "native_project_file_paths" => new NativeFilePaths(NativeFilesClient.SchemaId, projectId!, session, args["paths"]!.AsArray().Select(p => new NativeFilePathPair(p!.GetValue<string>(), p!.GetValue<string>(), "C:/work/" + p!.GetValue<string>())).ToArray()),
                "native_project_file_search" => new NativeFileSearchHit[] { new("nested/sample.csv", "sample.csv", false, 3) },
                "native_project_file_read" => new NativeFilePreview(NativeFilesClient.SchemaId, projectId!, session, "local", args["path"]!.GetValue<string>(), new(args["path"]!.GetValue<string>(), "text/csv", "a,b", null, false, 3)),
                "native_project_file_save" or "native_project_file_action" => true,
                "native_conversation_snapshot" => new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", ++sequence, projectId!, "created", [], null, false, false, false, "model", null, null, [], ComposerReferences: true, PlanMode: true),
                "native_runtimes_read" => new NativeRuntimeSnapshot(projectId!, session, [new("local", "local", "Local", "{}", "{}", null, null)], [], null, [], false),
                "list_ssh_hosts" => new JsonArray(new JsonObject { ["alias"] = "cluster" }),
                "list_models" => new JsonArray(), _ => null };
            if (command == "native_conversation_create") { Creates++; result = "created"; }
            return Task.FromResult(result is JsonNode node ? node : JsonSerializer.SerializeToNode(result, ConversationSnapshot.JsonOptions));
        }
    }
}
