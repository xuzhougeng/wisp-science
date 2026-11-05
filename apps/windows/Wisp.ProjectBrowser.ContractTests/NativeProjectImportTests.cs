using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProjectImportTests
{
    private static void Check(bool ok, string message) { if (!ok) throw new InvalidOperationException(message); }
    public static async Task RunAsync(string fixturePath)
    {
        var fixture = JsonNode.Parse(File.ReadAllText(fixturePath))!;
        Task<string?> Folder() => Task.FromResult<string?>("C:\\Research data\\study");
        Task<string?> Archive() => Task.FromResult<string?>("C:\\Exports\\study.zip");
        Task<string?> Cancel() => Task.FromResult<string?>(null);
        var cancelClient = new Fake(fixture);
        using var cancelled = new NativeProjectImportModel(cancelClient);
        Check(await cancelled.ChooseAsync(NativeProjectImportKind.Directory, Cancel, Folder) && cancelled.Stage == NativeProjectImportStage.Cancelled && cancelClient.Calls.Count == 0, "picker cancellation never imports");
        using var cancelledDestination = new NativeProjectImportModel(cancelClient);
        await cancelledDestination.ChooseAsync(NativeProjectImportKind.Archive, Archive, Cancel);
        Check(cancelledDestination.Stage == NativeProjectImportStage.Cancelled && cancelClient.Calls.Count == 0, "destination cancellation leaves the archive untouched");

        var directoryClient = new Fake(fixture); using var directory = new NativeProjectImportModel(directoryClient);
        Check(await directory.ChooseAsync(NativeProjectImportKind.Directory, Folder, () => throw new InvalidOperationException("unexpected destination")) && directory.ImportedProjectId == "imported", "directory import opens the returned project");
        Check(directoryClient.Calls.Single().Command == "native_project_import_directory" && directoryClient.Calls[0].Args["directory_path"]!.GetValue<string>() == await Folder(), "folder import uses the explicit picked folder");
        var zipClient = new Fake(fixture); using var zip = new NativeProjectImportModel(zipClient);
        Check(await zip.ChooseAsync(NativeProjectImportKind.Archive, Archive, Folder) && zip.Stage == NativeProjectImportStage.Complete && !zip.IsModal, "ZIP completion is a nonmodal status");
        Check(zipClient.Calls.Single().Args["destination_parent"]!.GetValue<string>() == await Folder() && zipClient.Calls[0].Args["archive_path"]!.GetValue<string>() == await Archive(), "ZIP import forwards source and chosen destination separately");

        var client = new Fake(fixture); using var recovery = new NativeProjectImportModel(client);
        Check(await recovery.ChooseAsync(NativeProjectImportKind.Recovery, Folder, Folder) && recovery.Stage == NativeProjectImportStage.Preview && recovery.CanRecover, "recovery reads a reviewable preview before writing");
        Check(client.Calls.Count == 1 && recovery.Count("recoverable_session_count") == 1 && recovery.Count("invalid_archive_count") == 1 && recovery.Timestamp("earliest_message_at") == 1700000000, "shared fixture counts and times remain intact");
        recovery.Name = " "; Check(!recovery.CanRecover && !await recovery.RecoverAsync(), "blank recovery names cannot submit");
        recovery.Name = "Recovered study"; client.Fail = true;
        Check(!await recovery.RecoverAsync() && recovery.Stage == NativeProjectImportStage.Preview && recovery.Name == "Recovered study", "failed recovery retains the review and name without retrying");
        Check(client.Calls.Count == 2, "ambiguous recovery is never replayed automatically");
        client.Fail = false; Check(await recovery.RecoverAsync() && recovery.ImportedProjectId == "recovered-1", "confirmed recovery opens only its returned ID");
        Check(client.Calls.Last().Args["workspace_dir"]!.GetValue<string>() == fixture["preview_result"]!["workspace_dir"]!.GetValue<string>(), "confirmation uses the canonical preview path");
        Check(client.Calls.All(call => call.Scope == null), "project import and recovery never inherit an active project");

        var emptyClient = new Fake(fixture); emptyClient.Preview["recoverable_session_count"] = 0L;
        using var empty = new NativeProjectImportModel(emptyClient); await empty.ChooseAsync(NativeProjectImportKind.Recovery, Folder, Folder);
        Check(!empty.CanRecover && !await empty.RecoverAsync() && emptyClient.Calls.Count == 1, "empty history is readable but cannot be recovered");
        var badClient = new Fake(fixture); badClient.Preview.Remove("message_count");
        using var bad = new NativeProjectImportModel(badClient); Check(!await bad.ChooseAsync(NativeProjectImportKind.Recovery, Folder, Folder) && bad.Stage == NativeProjectImportStage.Failed && !bad.CanRecover, "malformed previews fail closed");

        var picker = new TaskCompletionSource<string?>(); var closedClient = new Fake(fixture); using var closed = new NativeProjectImportModel(closedClient);
        var pending = closed.ChooseAsync(NativeProjectImportKind.Directory, () => picker.Task, Folder);
        Check(!await closed.ChooseAsync(NativeProjectImportKind.Directory, Folder, Folder), "duplicate imports are blocked while picking");
        closed.Dispose(); picker.SetResult(await Folder()); await pending;
        Check(closedClient.Calls.Count == 0, "a late picker result after disposal never starts a write");
        var delayedClient = new Fake(fixture) { Pending = new() }; using var delayed = new NativeProjectImportModel(delayedClient);
        var late = delayed.ChooseAsync(NativeProjectImportKind.Directory, Folder, Folder); delayed.Dispose(); delayedClient.Pending.SetResult(JsonNode.Parse("""{"id":"late"}""")); await late;
        Check(delayed.ImportedProjectId == null, "late mutation replies cannot navigate a closed import flow");
    }
    private sealed class Fake(JsonNode fixture) : INativeSettingsClient
    {
        public JsonObject Preview { get; } = (JsonObject)fixture["preview_result"]!.DeepClone();
        public bool Fail; public TaskCompletionSource<JsonNode?>? Pending;
        public List<(string Command, JsonObject Args, string? Scope)> Calls { get; } = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (Fail) throw new IOException("offline"); if (Pending != null) return Pending.Task;
            return Task.FromResult(command == "native_project_recovery_preview" ? Preview.DeepClone() : command == "native_project_recover_workspace" ? fixture["recover_result"]!.DeepClone() : JsonNode.Parse("""{"id":"imported"}"""));
        }
    }
}
