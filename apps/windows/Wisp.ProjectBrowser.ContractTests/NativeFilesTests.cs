using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeFilesTests
{
    public static async Task RunAsync(string fixtureDirectory)
    {
        var projectFixture = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtureDirectory, "../../native-files/v1/project.json")))!;
        var projectLocations = projectFixture["locations"]!.Deserialize<NativeFileLocations>(ConversationSnapshot.JsonOptions)!;
        var projectDirectory = projectFixture["directory"]!.Deserialize<NativeFileDirectory>(ConversationSnapshot.JsonOptions)!;
        var projectPaths = projectFixture["paths"]!.Deserialize<NativeFilePaths>(ConversationSnapshot.JsonOptions)!;
        Check(projectLocations.SessionId == null && projectDirectory.SessionId == null && projectPaths.SessionId == null
            && projectDirectory.Entries.Single().Name == "sample.csv", "shared project fixture needs no synthetic conversation");
        var fixture = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtureDirectory, "../../native-files/v1/browser.json")))!.AsObject();
        var host = new Host(fixture); var client = new NativeFilesClient(host);
        var locations = await client.LocationsAsync("research-1", "session-1");
        Check(locations.Locations[1].Kind == "ssh" && locations.LocalRoot.Contains("exploration-1"), "shared locations and active exploration root");
        var directory = await client.DirectoryAsync("research-1", "session-1", "local", "results");
        Check(directory.Entries[2].ModifiedUnixMillis == null, "older entries without timestamps stay compatible");
        Check(NativeFileListPresentation.VisibleFiles(directory.Entries, "", "size").Select(f => f.Name)
            .SequenceEqual(new[] { "figures", "notes.md", "QC.csv" }), "directories first, then size descending");
        Check(NativeFileListPresentation.VisibleFiles(directory.Entries, "", "modified").Select(f => f.Name)
            .SequenceEqual(new[] { "figures", "QC.csv", "notes.md" }), "missing dates sort last");
        Check(WorkspacePanelModel.RemoteParent("/home") == "/" && WorkspacePanelModel.RemoteParent("~/results") == "~"
            && WorkspacePanelModel.RemoteParent("/") == "/", "remote navigation preserves POSIX roots");
        host.Foreign = true;
        await Reject(() => client.LocationsAsync("research-1", "session-1"), "foreign project response rejected");
        host.Foreign = false; host.WrongContext = true;
        await Reject(() => client.ReadAsync("research-1", "session-1", "local", "results/QC.csv"), "foreign location preview rejected");
        host.WrongContext = false;
        Check(NativeFileListPresentation.DownloadName("CON.csv") == "_CON.csv"
            && NativeFileListPresentation.DownloadName("sample:1?.csv") == "sample_1_.csv", "remote names are safe save suggestions");
        var hits = await client.SearchAsync("research-1", "session-1", "QC");
        Check(hits.Single().Path == "nested/QC.csv", "recursive search preserves root-relative paths");
        host.SearchPath = "../QC.csv";
        await Reject(() => client.SearchAsync("research-1", "session-1", "QC"), "escaping search path rejected");
        host.SearchPath = "nested/QC.csv";

        WorkspacePanelModel Model() => new(new NativePanelClient(host), "research-1", "session-1",
            new NativePanelTabs("[\"files\",\"artifacts\"]", "files", ["files", "artifacts"]), files: client);
        var model = Model(); await model.RefreshAsync("files", "results");
        var pendingSearch = host.SearchPending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var oldSearch = model.SearchFilesAsync("old"); host.SearchPending = null;
        await model.SearchFilesAsync("QC"); pendingSearch.SetResult(new JsonArray()); await oldSearch;
        Check(model.FileSearchQuery == "QC" && model.FileSearchHits.Single().Path == "nested/QC.csv", "late search cannot replace newer results");
        Check(!model.CanUploadFiles && !model.CanMutateLocalFiles, "search results cannot target current-directory mutations");
        pendingSearch = host.SearchPending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        oldSearch = model.SearchFilesAsync("QC"); host.SearchPending = null;
        await model.RefreshAsync("files", "results"); pendingSearch.SetResult(new JsonArray()); await oldSearch;
        Check(model.FileSearchQuery == "" && !model.FileSearchLoading, "navigation clears pending search");
        Check(model.CanUploadFiles && model.CanMutateLocalFiles, "local writable controls available after scoped load");
        var stalePicker = model.CaptureFileDestination()!;
        await model.SelectFileLocationAsync("ssh:lab");
        await model.UploadFilesAsync(stalePicker, ["/incoming/QC.csv"]);
        Check(host.Uploads == 0 && model.Path == "/home/research/results" && !model.CanMutateLocalFiles, "late picker cannot upload to another source; remote canonical path used");
        await model.ReadFileAsync("/home/research/results/QC.csv"); model.EditPreview();
        await model.SaveFileAsync("old", "new");
        Check(model.Preview?.Path.StartsWith("ssh://") == true && !model.CanEditPreview && !model.PreviewEditable
            && host.Saves == 0, "remote preview keeps reference and cannot save through local API");
        await model.SelectFileLocationAsync("local");
        Check(model.Path == "results" && model.Preview == null, "return to local directory clears remote preview");
        var pending = host.DirectoryPending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var refresh = model.RefreshAsync("files", "old");
        host.DirectoryPending = null; await model.SelectFileLocationAsync("ssh:lab");
        pending.SetResult(host.Reply("local_directory", "local", "old")); await refresh;
        Check(model.RemoteFiles && model.Path == "/home/research/results", "late local listing cannot overwrite remote source");
        await model.SelectFileLocationAsync("local");
        host.FailUpload = true;
        await model.UploadFilesAsync(model.CaptureFileDestination()!, ["/incoming/QC.csv"]);
        Check(model.FileTransferUncertain && !model.CanUploadFiles && host.Uploads == 1, "lost transfer response blocks replay");
        await model.UploadFilesAsync(stalePicker, ["/incoming/QC.csv"]);
        Check(host.Uploads == 1, "uncertain request never auto retried");
        await model.RefreshAsync("files"); host.FailUpload = false;
        await model.UploadFilesAsync(model.CaptureFileDestination()!, ["/incoming/QC.csv", "/incoming/missing.csv"]);
        Check(model.FileTransfer?.Items.Count(i => i.Status == "failed") == 1 && model.CanUploadFiles && !model.FileTransferBusy,
            "partial success retained after refresh and busy flag released");
        await model.ReadFileAsync("results/QC.csv"); model.EditPreview();
        host.ReadOnly = true; await model.RefreshAsync("files");
        Check(!model.CanUploadFiles && !model.CanMutateLocalFiles && !model.CanEditPreview && !model.PreviewEditable,
            "archived/read-only scope disables mutations and an already-open editor");
        host.ReadOnly = false; await model.RefreshAsync("files");
        var closedPicker = model.CaptureFileDestination()!; model.Close();
        await model.UploadFilesAsync(closedPicker, ["/incoming/QC.csv"]);
        Check(host.Uploads == 2, "closing while OS picker is open prevents a late upload");
        var remote = Model(); host.ReadOnly = true; await remote.RefreshAsync("files"); await remote.SelectFileLocationAsync("ssh:lab");
        var downloadSource = remote.CaptureFileDownload()!;
        Check(downloadSource != null && !remote.CanUploadFiles, "read-only remote scope still allows inbound download");
        await remote.DownloadFileAsync(downloadSource!, "/home/research/results/QC.csv", null);
        Check(host.Downloads == 0, "cancelled save picker does not submit download");
        await remote.DownloadFileAsync(downloadSource!, "/home/research/results/QC.csv", "/exports/QC.csv");
        Check(host.Downloads == 1 && remote.HasPendingFileTransfers && !remote.CanDownloadFiles, "running download is not complete or resubmitted");
        host.RunStatus = "succeeded"; host.WrongRun = true; await remote.RefreshFileTransfersAsync();
        Check(remote.HasPendingFileTransfers && remote.FileTransferReadError != null, "foreign run cannot report successful download");
        host.WrongRun = false; await remote.RefreshFileTransfersAsync();
        Check(remote.FileTransfer!.Items.Single().Status == "succeeded" && remote.CanDownloadFiles, "confirmed terminal run updates download state");
        var lateSave = remote.CaptureFileDownload()!; await remote.SelectFileLocationAsync("local");
        await remote.DownloadFileAsync(lateSave, "/home/research/results/QC.csv", "/exports/QC.csv");
        Check(host.Downloads == 1, "late save cannot download from a different source");
        await remote.SelectFileLocationAsync("ssh:lab");
        await remote.DownloadFileAsync(remote.CaptureFileDownload()!, "/home/research/results/QC.csv", "/exports/QC.csv");
        var runPending = host.RunPending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var runRead = remote.RefreshFileTransfersAsync(); remote.Close();
        runPending.SetResult(host.RunReply()); await runRead;
        Check(remote.FileTransfer!.Items.Single().Status == "running", "closing panel rejects a late terminal transfer read");
        Check(host.Calls.All(c => c.Project == "research-1" && c.Args["session_id"]?.GetValue<string>() == "session-1"), "all requests preserve session scope");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static async Task Reject(Func<Task> action, string message)
    { try { await action(); } catch (InvalidDataException) { return; } throw new InvalidOperationException(message); }
    private sealed class Host(JsonObject fixture) : INativeSettingsClient
    {
        public bool Foreign, WrongContext, ReadOnly, FailUpload, WrongRun;
        public string SearchPath = "nested/QC.csv", RunStatus = "running";
        public int Uploads, Saves, Downloads;
        public TaskCompletionSource<JsonNode?>? DirectoryPending, SearchPending, RunPending;
        public JsonObject RunReply() => new() {
            ["id"] = WrongRun ? "foreign" : "download-1", ["frame_id"] = "session-1", ["context_id"] = "ssh:lab",
            ["title"] = "Download", ["kind"] = "file_transfer", ["status"] = RunStatus, ["created_at"] = 0, ["progress_json"] = "{}" };
        public readonly List<(string Command, string? Project, JsonObject Args)> Calls = [];
        public JsonObject Reply(string key, string? context = null, string? path = null)
        {
            var result = fixture[key]!.DeepClone().AsObject();
            if (Foreign) result["project_id"] = "foreign";
            if (context != null) result["context_id"] = WrongContext ? "foreign" : context;
            if (path != null) result[key.Contains("preview") ? "requested_path" : "path"] = path;
            if (key == "locations") result["read_only"] = ReadOnly;
            return result;
        }
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, projectId, args.DeepClone().AsObject()));
            var context = args["context_id"]?.GetValue<string>() ?? "local";
            var path = args["path"]?.GetValue<string>() ?? ".";
            if (command.EndsWith("searchfiles")) return SearchPending?.Task ?? Task.FromResult<JsonNode?>(new JsonArray(new JsonObject {
                ["path"] = SearchPath, ["name"] = "QC.csv", ["is_dir"] = false, ["size"] = 17 }));
            if (command.EndsWith("_download")) { Downloads++; return Task.FromResult<JsonNode?>(Reply("remote_download", context, path)); }
            if (command.EndsWith("run_detail")) return RunPending?.Task ?? Task.FromResult<JsonNode?>(RunReply());
            if (command.EndsWith("_locations")) return Task.FromResult<JsonNode?>(Reply("locations"));
            if (command.EndsWith("_directory")) {
                if (DirectoryPending != null) return DirectoryPending.Task;
                return Task.FromResult<JsonNode?>(Reply(context == "local" ? "local_directory" : "remote_directory", context, context == "local" ? path : "/home/research/results"));
            }
            if (command.EndsWith("_read")) return Task.FromResult<JsonNode?>(Reply(context == "local" ? "local_preview" : "remote_preview", context, path));
            if (command.EndsWith("_upload")) {
                Uploads++; if (FailUpload) throw new IOException("reply lost");
                return Task.FromResult<JsonNode?>(Reply(context == "local" ? "local_upload" : "remote_upload", context, path));
            }
            if (command.EndsWith("savefile")) Saves++;
            return Task.FromResult<JsonNode?>(JsonValue.Create(true));
        }
    }
}
