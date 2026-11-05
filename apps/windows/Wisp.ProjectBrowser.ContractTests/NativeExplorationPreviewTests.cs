using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeExplorationPreviewTests
{
    public static async Task RunAsync(string fixtures)
    {
        var data = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtures, "../../native-conversations/v1/exploration-preview.json")))!;
        var owner = NativeExplorations.Decode(JsonNode.Parse(File.ReadAllText(Path.Combine(fixtures, "../../native-conversations/v1/explorations.json"))), "research-1").Explorations[0];
        var decoded = NativeExplorationPreview.Decode(data, "research-1", owner);
        var preview = decoded.Preview;
        Check(Enum.GetValues<ExplorationReviewTab>().Select(tab => WorkspaceExplorationPreview.Count(preview.Diff, tab)).SequenceEqual([3, 1, 1, 2, 2]), "tab counts include edges/resources, exclude mainline files");
        var files = WorkspaceExplorationPreview.Sections(preview, ExplorationReviewTab.Files);
        Check(files.Length == 2 && files[0].Rows.Length == 3 && files[1].Rows.Single().Title == "notes/mainline.md", "separate candidate and mainline file changes");
        Check(WorkspaceExplorationPreview.Sections(preview, ExplorationReviewTab.Artifacts)[0].Rows.Single().Detail == "version-2", "artifact tab keeps version");
        Check(WorkspaceExplorationPreview.Sections(preview, ExplorationReviewTab.Decisions)[0].Rows[1].Detail == "decision-1 → run-1", "decision edges retain endpoints");
        Check(WorkspaceExplorationPreview.Sections(preview, ExplorationReviewTab.ExternalEffects)[0].Rows[0].Warning
            && WorkspaceExplorationPreview.Sections(preview, ExplorationReviewTab.ExternalEffects)[0].Rows[1].Detail.StartsWith("ssh://"), "external effects and references are separate text rows");
        foreach (var mode in new[] { "project", "source", "frame", "diff", "eligible", "manual", "hash", "files" })
        {
            var invalid = data.DeepClone();
            if (mode == "project") invalid["project_id"] = "other";
            if (mode == "source") invalid["source_session_id"] = "other";
            if (mode == "frame") invalid["preview"]!["exploration"]!["frame_id"] = "other";
            if (mode == "diff") invalid["preview"]!["diff"]!["explorationId"] = "other";
            if (mode is "eligible" or "manual") invalid["preview"]!["eligibility"]!.AsObject().Remove(mode == "eligible" ? "eligible" : "manualResolutionAvailable");
            if (mode == "hash") invalid["preview"]!["eligibility"]!["expectedGuardHash"] = "";
            if (mode == "files") invalid["preview"]!["diff"]!["files"] = null;
            Reject(() => NativeExplorationPreview.Decode(invalid, "research-1", owner));
        }
        Check(NativeSessionMenuEntries.Exploration(owner).Select(row => row.Id).SequenceEqual(["open", "promote", "preview", "discard"]), "active menu matches WebView order");
        Check(NativeSessionMenuEntries.Exploration(owner with { Exploration = owner.Exploration with { Status = "creating" } }).Select(row => row.Id).SequenceEqual(["open", "preview"]), "creating menu omits mutation entries");
        var host = new Host(data); using var model = new WorkspaceExplorationPreview(host, "research-1", owner);
        Check(!model.CanPromote && await model.LoadAsync() && model.CanPromote && model.CanDiscard && !model.CanResolveManually, "review gates actions");
        Check(await model.CompleteAsync("promote") && model.Result?.SourceSessionId == "main-1", "confirmed promotion returns exact source");
        Check(host.LastArgs?["expected_guard_hash"]?.ToString() == "guard-1", "promotion uses reviewed guard");
        var count = host.Calls; Check(!await model.CompleteAsync("promote") && host.Calls == count, "no successful replay");
        host.Blocked = true; using var manual = new WorkspaceExplorationPreview(host, "research-1", owner);
        await manual.LoadAsync(); Check(!manual.CanPromote && manual.CanResolveManually, "blocked preview offers explicit manual path");
        Check(await manual.OpenFoldersAsync() && manual.Result == null, "opening resolved folders does not complete round");
        Check(await manual.CompleteAsync("manual_finish") && host.LastArgs?.Count == 2 && manual.Result?.Action == "manual_finish", "manual completion uses candidate/source scope, no arbitrary path");
        host.Blocked = false; host.Fail = true; using var failed = new WorkspaceExplorationPreview(host, "research-1", owner);
        await failed.LoadAsync(); Check(!await failed.CompleteAsync("discard") && failed.MutationUnconfirmed && !failed.CanDiscard, "ambiguous action blocks replay");
        count = host.Calls; await failed.CompleteAsync("discard"); Check(host.Calls == count, "failed action is never replayed");
        host.Fail = false; await failed.LoadAsync(); Check(!failed.CanPromote && failed.ShowFailure && failed.Error == null, "successful rereading retains visible mutation uncertainty");
        foreach (var wrong in new[] { "project_id", "session_id", "source_session_id", "exploration_id", "action", "completed" })
        {
            host.Wrong = wrong; using var invalid = new WorkspaceExplorationPreview(host, "research-1", owner); await invalid.LoadAsync();
            Check(!await invalid.CompleteAsync("discard") && invalid.MutationUnconfirmed && invalid.Result == null, "invalid action acknowledgement cannot complete");
        }
        host.Wrong = null; using var busy = new WorkspaceExplorationPreview(host, "research-1", owner); await busy.LoadAsync(); host.Pending = new();
        var pending = busy.CompleteAsync("promote"); Check(!busy.CanDismiss && !busy.CanRead && busy.Busy, "pending mutation locks dismissal and reread");
        count = host.Calls; await busy.CompleteAsync("promote"); Check(host.Calls == count, "pending action submits once");
        busy.Dispose(); host.Pending.SetResult(host.Reply("promote")); Check(!await pending && busy.Result == null, "closed surface ignores late acknowledgement");
        host.Pending = new(); using var closed = new WorkspaceExplorationPreview(host, "research-1", owner); var read = closed.LoadAsync(); closed.Dispose(); host.Pending.SetResult(data.DeepClone());
        Check(!await read && closed.Preview == null, "closed preview ignores late read");
        var catalog = System.Text.Json.JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(File.ReadAllText(Path.Combine(fixtures, "../../../apps/macos/Sources/WispProjectBrowserUI/Resources/native-locales.json")))!;
        var locale = new NativeLocalization(catalog, "en");
        Check(WorkspaceExplorationPreview.BlockerText(new("MainlineAdvanced", "raw detail"), locale) != "raw detail", "known blockers use shared localization");
        Check(WorkspaceExplorationPreview.BlockerText(new("FutureCode", "keep explanation"), locale) == "keep explanation", "unknown blockers retain backend explanation");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static void Reject(Action action) { try { action(); } catch (Exception e) when (e is InvalidDataException or System.Text.Json.JsonException) { return; } throw new InvalidOperationException("Invalid preview accepted"); }
    private sealed class Host(JsonNode data) : INativeSettingsClient
    {
        public bool Blocked, Fail; public string? Wrong; public int Calls; public JsonObject? LastArgs; public TaskCompletionSource<JsonNode?>? Pending;
        public JsonNode Reply(string action)
        {
            var reply = new JsonObject { ["project_id"] = "research-1", ["session_id"] = "private-1", ["source_session_id"] = "main-1", ["exploration_id"] = "explore-1", ["action"] = action, ["completed"] = true };
            if (Wrong == "completed") reply.Remove("completed"); else if (Wrong != null) reply[Wrong] = "other";
            return reply;
        }
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(projectId == "research-1" && args["session_id"]?.ToString() == "private-1" && args["exploration_id"]?.ToString() == "explore-1", "every command carries explicit reviewed scope");
            Calls++; LastArgs = args.DeepClone().AsObject(); if (Pending != null) return Pending.Task;
            if (command.EndsWith("_preview"))
            {
                var result = data.DeepClone(); if (Blocked) { var eligibility = result["preview"]!["eligibility"]!; eligibility["eligible"] = false; eligibility["manualResolutionAvailable"] = true; eligibility["reasons"] = new JsonArray(new JsonObject { ["code"] = "MainlineAdvanced", ["message"] = "main changed" }); }
                return Task.FromResult<JsonNode?>(result);
            }
            if (Fail) throw new IOException("Lost acknowledgement");
            return Task.FromResult<JsonNode?>(Reply(command["native_conversation_exploration_".Length..]));
        }
    }
}
