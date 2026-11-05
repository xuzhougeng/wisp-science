using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProjectSettingsTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var home = new NativeProjectSettingsModel(fake, "p", false);
        Check(await home.LoadAsync() && home.Name == "Project" && home.AgentContext == "original\ncontext", "home settings load the existing multiline fields");
        Check(fake.Calls.Count == 1 && fake.Calls[0].Args["id"]!.GetValue<string>() == "p" && fake.Calls[0].Context == "p", "home lookup carries explicit identity without activation or retention reads");
        home.Name = "  "; Check(!home.CanSave && !await home.SaveAsync(), "blank names cannot be saved");
        home.Name = "Renamed"; home.AgentContext += "\nnew rule";
        Check(!await home.SaveAsync() && home.ConfirmationOpen && fake.Calls.Count == 1, "changed context opens confirmation before any write");
        home.CancelConfirmation(); Check(!home.ConfirmationOpen && home.AgentContext.EndsWith("new rule"), "Escape dismisses only confirmation and retains the draft");
        Check(!await home.SaveAsync(confirmContext: true) && home.ConfirmationOpen && fake.Calls.Count == 1, "a confirmation flag alone cannot bypass the visible confirmation step");
        Check(await home.SaveAsync(confirmContext: true) && !home.ConfirmationOpen, "confirmed context writes once");
        var write = fake.Calls.Last(); Check(write.Command == "update_project" && write.Args["id"]!.GetValue<string>() == "p" && write.Args["agentContext"]!.GetValue<string>().EndsWith("new rule") && !write.Args.ContainsKey("agent_context"), "save keeps project ID and camelCase payload");
        home.AgentContext = " \n" + home.AgentContext + "\n "; Check(!home.ContextChanged, "surrounding whitespace does not add a context confirmation");
        Check(await home.EnableFolderSyncAsync() && home.FolderSync && !await home.EnableFolderSyncAsync(), "folder sync is a one-way acknowledged action and is not replayed");

        var sync = new Fake { SyncFailure = new NativeSettingsCommandException("The folder is read-only.") };
        using var cloud = new NativeProjectSettingsModel(sync, "p", false, localization: new(new Dictionary<string, Dictionary<string, string>>(), "en"));
        await cloud.LoadAsync(); cloud.Description = "my draft";
        Check(!await cloud.EnableFolderSyncAsync() && cloud.FolderSyncError == "The folder is read-only."
            && cloud.Error == null && !cloud.FolderSync && cloud.CanSave && cloud.Description == "my draft",
            "a rejected cloud-folder action displays only its own error and preserves the editable form");
        sync.SyncFailure = new IOException("connection lost");
        Check(!await cloud.EnableFolderSyncAsync() && cloud.FolderSyncError?.Contains("will not be retried automatically") == true
            && cloud.Error == null && !cloud.FolderSync && sync.Calls.Count(call => call.Command == "enable_project_folder_sync") == 2,
            "a missing reply to an explicit retry is reported inline in the active language without replay");
        sync.SyncFailure = null;
        sync.DelayedSync = new();
        var enabling = cloud.EnableFolderSyncAsync();
        Check(cloud.EnablingFolderSync && !cloud.Busy && !cloud.CanSave && cloud.FolderSyncError == null
            && !await cloud.EnableFolderSyncAsync() && !await cloud.SaveAsync(), "a pending folder action does not lock the editors or allow duplicate writes");
        cloud.Description = "typed while waiting"; sync.DelayedSync.SetResult(null);
        Check(await enabling && cloud.FolderSync && cloud.FolderSyncError == null && cloud.Description == "typed while waiting",
            "an acknowledged retry clears the folder error and retains unsaved edits");
        var late = new Fake { DelayedSync = new() };
        using var left = new NativeProjectSettingsModel(late, "p", false);
        await left.LoadAsync(); var finished = left.EnableFolderSyncAsync(); left.Dispose();
        late.DelayedSync.SetException(new IOException("disconnected"));
        Check(!await finished && !left.FolderSync && left.FolderSyncError == null, "late sync replies cannot reopen a dismissed project form");

        var windows = new Fake { Delayed = new() };
        using var edited = new NativeProjectSettingsModel(windows, "p", false);
        var stored = Fake.Project(); stored["agent_context"] = "line one\r\nline two";
        windows.Delayed.SetResult(stored); await edited.LoadAsync();
        edited.AgentContext = "line one\nchanged";
        Check(!await edited.SaveAsync() && edited.ConfirmationOpen, "real changes to multi-line Windows context still require confirmation");
        edited.CancelConfirmation(); edited.AgentContext = "line one\nline two"; edited.Description = "new description";
        Check(await edited.SaveAsync() && !edited.ConfirmationOpen && windows.Calls.Count == 2
            && windows.Calls.Last().Args["agentContext"]!.GetValue<string>() == "line one\r\nline two",
            "edit/undo followed by saving another field neither confirms nor rewrites the stored line endings");

        var other = new Fake(); using var workspace = new NativeProjectSettingsModel(other, "p", true);
        Check(await workspace.LoadAsync() && workspace.RetentionReady && workspace.Retention.SequenceEqual(new[] { "", "30", "7" }), "workspace dialog reads per-project retention with nullable off values");
        Check(!await workspace.EnableFolderSyncAsync(), "workspace dialog exposes retention rather than the home cloud-folder action");
        Check(!await workspace.SaveRetentionAsync(["0", "30", "7"]) && workspace.RetentionInvalid && other.Calls.Count == 2, "invalid retention does not issue a cleanup-policy mutation");
        Check(await workspace.SaveRetentionAsync(["", "30", "7"]) && other.Calls.Count == 2, "unchanged retention is not written merely by moving focus");
        Check(await workspace.SaveRetentionAsync(["3650", "", "1"]), "valid changed retention can be saved independently");
        var retention = other.Calls.Last(); Check(retention.Command == "set_project_run_retention" && retention.Context == "p" && retention.Args["runRetentionDays"]!.GetValue<long>() == 3650 && retention.Args["failedRunRetentionDays"] == null, "retention preserves scope, limits and null-off semantics");
        Check(!NativeProjectSettingsModel.TryRetention(["3651", "", ""], out _) && !NativeProjectSettingsModel.TryRetention(["1.5", "", ""], out _), "out-of-range and fractional retention values are rejected");

        var stepped = new Fake();
        using var days = new NativeProjectSettingsModel(stepped, "p", true);
        await days.LoadAsync();
        var newDays = NativePreferenceInteger.Step(days.Retention[0], 1, 1, 3650);
        Check(newDays == "1" && await days.SaveRetentionAsync([newDays, "30", "7"]) && await days.SaveRetentionAsync([newDays, "30", "7"])
            && stepped.Calls.Count(call => call.Command == "set_project_run_retention") == 1,
            "stepping from Off commits one day immediately; subsequent blur does not repeat the write");
        Check(await days.SaveRetentionAsync(["", "30", "7"]) && stepped.Calls.Last().Args["runRetentionDays"] == null,
            "clearing a stepped value turns cleanup off again");

        var delayedRetention = new Fake { DelayedRetention = new() };
        using var pendingRetention = new NativeProjectSettingsModel(delayedRetention, "p", true);
        await pendingRetention.LoadAsync();
        var retentionSave = pendingRetention.SaveRetentionAsync(["14", "30", "7"]);
        Check(pendingRetention.RetentionSaving && !pendingRetention.Busy && pendingRetention.CanSave, "retention autosave does not disable the project form or move keyboard focus");
        Check(!await pendingRetention.SaveRetentionAsync(["21", "30", "7"]), "pending retention writes cannot be duplicated");
        delayedRetention.DelayedRetention.SetResult(null);
        Check(await retentionSave && !pendingRetention.RetentionSaving && pendingRetention.Retention[0] == "14", "acknowledged retention updates its independent baseline");

        var lateRetention = new Fake { DelayedRetention = new() };
        using var dismissedRetention = new NativeProjectSettingsModel(lateRetention, "p", true);
        await dismissedRetention.LoadAsync();
        var lateSave = dismissedRetention.SaveRetentionAsync(["14", "30", "7"]);
        dismissedRetention.Dispose(); lateRetention.DelayedRetention.SetResult(null);
        Check(!await lateSave && dismissedRetention.Retention[0] == "", "closing the dialog suppresses late retention replies");

        var restarted = new Fake { Disconnected = true };
        var connections = 0;
        using var recovered = new NativeProjectSettingsModel(restarted, "p", true, _ =>
        { connections++; restarted.Disconnected = false; return Task.CompletedTask; });
        Check(await recovered.LoadAsync() && recovered.RetentionReady && connections == 1
            && restarted.Calls.Select(c => c.Command).SequenceEqual(new[] { "get_project_settings", "get_project_settings", "get_project_run_retention" })
            && restarted.Calls.All(c => c.Context == "p"), "reopening against a stale endpoint reconnects once and reads the same project");
        restarted.Disconnected = true; recovered.Description = "keep my edit";
        Check(!await recovered.SaveAsync() && connections == 1 && recovered.Description == "keep my edit"
            && restarted.Calls.Count(c => c.Command == "update_project") == 1, "a lost connection during save is never reconnected and replayed");
        var gate = new TaskCompletionSource(); var missing = new Fake { Disconnected = true };
        using var dismissed = new NativeProjectSettingsModel(missing, "p", false, _ => gate.Task);
        var reopening = dismissed.LoadAsync(); dismissed.Dispose(); gate.SetResult();
        Check(!await reopening && !dismissed.Loaded && missing.Calls.Count == 1, "closing before reconnection completes cannot start another project read");

        var wrong = new Fake { WrongId = true }; using var mismatch = new NativeProjectSettingsModel(wrong, "p", false);
        Check(!await mismatch.LoadAsync() && !mismatch.Loaded && !mismatch.CanSave, "mismatched project responses cannot create a writable form");
        var failure = new Fake { FailUpdate = true }; using var failed = new NativeProjectSettingsModel(failure, "p", false);
        await failed.LoadAsync(); failed.Description = "keep draft";
        Check(!await failed.SaveAsync() && failed.Description == "keep draft" && failure.Calls.Count(call => call.Command == "update_project") == 1, "failed saves preserve values without automatic replay");
        var pending = new Fake { Delayed = new() }; using var closed = new NativeProjectSettingsModel(pending, "p", false);
        var load = closed.LoadAsync(); closed.Dispose(); pending.Delayed.SetResult(Fake.Project()); await load;
        Check(!closed.Loaded && !closed.CanSave && !await closed.SaveAsync(), "closing during a read suppresses its late reply and future writes");
        Check(fake.Calls.Concat(other.Calls).All(call => !call.Command.Contains("activate") && !call.Command.Contains("open_project")), "project editing does not activate the inspected workspace");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool WrongId, FailUpdate, Disconnected;
        public Exception? SyncFailure;
        public TaskCompletionSource<JsonNode?>? Delayed, DelayedRetention, DelayedSync;
        public List<(string Command, JsonObject Args, string? Context)> Calls { get; } = [];
        public static JsonNode Project() => JsonNode.Parse("""{"id":"p","name":"Project","description":"description","agent_context":"original\ncontext","folder_sync":false}""")!;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (Disconnected) throw new HttpRequestException("connection refused");
            if (command == "get_project_settings") { if (Delayed != null) return Delayed.Task; var value = Project(); if (WrongId) value["id"] = "another"; return Task.FromResult<JsonNode?>(value); }
            if (command == "get_project_run_retention") return Task.FromResult(JsonNode.Parse("""{"run_retention_days":null,"failed_run_retention_days":30,"orphan_file_retention_days":7}"""));
            if (command == "set_project_run_retention" && DelayedRetention != null) return DelayedRetention.Task;
            if (command == "update_project" && FailUpdate) throw new IOException("offline");
            if (command == "enable_project_folder_sync" && SyncFailure != null) throw SyncFailure;
            if (command == "enable_project_folder_sync" && DelayedSync != null) return DelayedSync.Task;
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
