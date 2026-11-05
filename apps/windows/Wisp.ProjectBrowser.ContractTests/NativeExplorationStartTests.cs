using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeExplorationStartTests
{
    public static async Task RunAsync(string fixtures)
    {
        var data = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtures, "../../native-conversations/v1/exploration-started.json")))!;
        var target = new NativeHistoryTarget("research-1", "main-1", new(3, 40, "turn-3"), "revision-1", "assistant", "");
        Check(NativeExplorationStarted.Decode(data, target).Exploration.FrameId == "private-1", "shared acknowledgement identifies private candidate");
        foreach (var mode in new[] { "project", "source", "identity", "checkpoint", "private", "status" })
        {
            var invalid = data.DeepClone();
            if (mode == "project") invalid["project_id"] = "other";
            if (mode == "source") invalid["summary"]!["source_frame_id"] = "other";
            if (mode == "identity") invalid["target"]!["digest"] = "replaced";
            if (mode == "checkpoint") invalid["summary"]!["checkpoint_user_index"] = 2;
            if (mode == "private") invalid["summary"]!["exploration"]!["frame_id"] = "main-1";
            if (mode == "status") invalid["summary"]!["exploration"]!["status"] = "failed";
            Reject(() => NativeExplorationStarted.Decode(invalid, target));
        }
        var host = new Host(data);
        using var form = new WorkspaceExplorationStart(host, target, "  ");
        Check(!form.CanSubmit && !await form.CreateAsync() && host.Creates == 0, "blank name never submits");
        form.Name = "  Alternative normalization  "; host.Pending = new(); var pending = form.CreateAsync();
        Check(form.Busy && !form.CanDismiss && !form.CanSubmit, "pending create locks dismiss/edit/replay");
        await form.CreateAsync(); Check(host.Creates == 1, "pending create is not duplicated");
        host.Pending.SetResult(data.DeepClone()); Check(await pending && form.Result?.Exploration.FrameId == "private-1", "valid completion carries private navigation");
        Check(host.LastArgs?["name"]?.ToString() == "Alternative normalization" && host.LastArgs?["target"]?.Deserialize<NativeTurnIdentity>(ConversationSnapshot.JsonOptions) == target.Turn, "trimmed name and captured identity sent together");
        Check(!await form.CreateAsync() && host.Creates == 1, "successful create cannot replay");
        host.Pending = null; host.Fail = true; using var failed = new WorkspaceExplorationStart(host, target, "Candidate");
        Check(!await failed.CreateAsync() && failed.Unconfirmed && failed.Error != null && failed.CanDismiss, "failed creation stays visible and closeable");
        var calls = host.Creates; failed.Name = "Changed"; Check(!await failed.CreateAsync() && calls == host.Creates, "renaming does not clear uncertain creation");
        host.Fail = false; host.Pending = new(); using var closed = new WorkspaceExplorationStart(host, target, "Candidate");
        pending = closed.CreateAsync(); closed.Dispose(); host.Pending.SetResult(data.DeepClone());
        Check(!await pending && closed.Result == null, "closed surface ignores late creation reply"); host.Pending = null;

        var conversation = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await conversation.OpenAsync("research-1", "main-1");
        var page = conversation.Snapshot!;
        Check(conversation.ExplorationTarget(page, 3)?.Turn == target.Turn && conversation.ExplorationTarget(page, 2) == null, "assistant-only entry uses absolute historical offset");
        Check(conversation.CanStartExploration(target), "idle ordinary conversation can start");
        host.State = host.State with { Running = true }; await conversation.RefreshAsync();
        Check(!conversation.CanStartExploration(target) && conversation.ExplorationTarget(conversation.Snapshot!, 3) == null
            && conversation.ExplorationTarget(conversation.Snapshot!, 1) != null, "running tail hidden; historical entry remains disabled");
        host.State = host.State with { Running = false, ReadOnly = true, Exploration = new(null, 1, true, "main-1", 3, 2) }; await conversation.RefreshAsync();
        Check(conversation.CanStartExploration(target) && conversation.CurrentExplorationRoundTarget?.Turn == target.Turn, "frozen source can add candidate at exact round checkpoint");
        Check(!conversation.CanStartExploration(target with { Turn = new(2, 30, "turn-2") }), "another checkpoint cannot join round");
        host.State = host.State with { ReadOnly = false, Exploration = new(null, 0, false, "other", 3, 0) }; await conversation.RefreshAsync();
        Check(!conversation.CanStartExploration(target), "other source cannot join current round");
        host.State = host.State with { Exploration = null, AcpAgentId = "acp" }; await conversation.RefreshAsync();
        Check(conversation.ExplorationTarget(conversation.Snapshot!, 3) == null && !conversation.CanStartExploration(target), "ACP entry hidden");
        host.State = host.State with { AcpAgentId = null, HistoryState = host.State.HistoryState! with { CanBranch = false } }; await conversation.RefreshAsync();
        Check(conversation.ExplorationTarget(conversation.Snapshot!, 3) == null, "branches and assistant scope hidden");
        var catalog = JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(File.ReadAllText(Path.Combine(fixtures, "../../../apps/macos/Sources/WispProjectBrowserUI/Resources/native-locales.json")))!;
        var locale = new NativeLocalization(catalog, "en");
        Check(WorkspaceExplorationStart.DefaultName(locale, 2) == "Exploration 3", "default name counts all source candidates");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static void Reject(Action action) { try { action(); } catch (Exception e) when (e is InvalidDataException or JsonException) { return; } throw new InvalidOperationException("Invalid creation accepted"); }
    private sealed class Host(JsonNode data) : INativeSettingsClient
    {
        public TaskCompletionSource<JsonNode?>? Pending; public bool Fail; public int Creates; public JsonObject? LastArgs; private ulong sequence;
        public ConversationSnapshot State = new(ConversationSnapshot.SchemaId, "synthetic", 1, "research-1", "main-1",
            [new("user", "Third", null, null, null, null), new("assistant", "Third response", null, null, null, null),
             new("user", "Fourth", null, null, null, null), new("assistant", "Fourth response", null, null, null, null)], null,
            false, false, false, "", null, null, [], UserOffset: 2,
            HistoryState: new("revision-1", true, false, Enumerable.Range(0,4).Select(i => new NativeTurnIdentity(i, (i+1)*10, "turn-"+i)).ToArray()));
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "native_conversation_exploration_start")
            {
                Check(projectId == "research-1" && args["session_id"]?.ToString() == "main-1", "creation is project/source scoped");
                Creates++; LastArgs = args.DeepClone().AsObject(); if (Fail) throw new IOException("Synthetic lost acknowledgement");
                return Pending?.Task ?? Task.FromResult<JsonNode?>(data.DeepClone());
            }
            if (command == "native_conversation_snapshot") return Task.FromResult(JsonSerializer.SerializeToNode(State with { Sequence = ++sequence }, ConversationSnapshot.JsonOptions));
            return Task.FromResult<JsonNode?>(command is "list_models" or "list_acp_agents" ? new JsonArray() : null);
        }
    }
}
