using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeConversationHistoryTests
{
    public static async Task RunAsync()
    {
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        Check(WorkspaceConversationModel.HistoryDraft("question\n\nUploaded files: old.tsv") == "question", "restored draft excludes stale attachment metadata");
        await model.OpenAsync("p", "s");
        Check(model.Snapshot != null, "legacy sequence-zero user identities are valid");
        var target = model.HistoryTarget(model.Snapshot!, 1)!;
        Check(target.Turn.UserIndex == 34 && target.Turn.UserSeq == 69, "paged assistant actions carry the absolute durable turn");
        Check(model.MessageActions(model.Snapshot!, 0).Select(action => action.Kind).SequenceEqual(["copy", "rewind", "branch", "excerpt"]),
            "user controls follow WebView's copy/rewind/branch/excerpt order");
        Check(model.MessageActions(model.Snapshot!, 1, true).Select(action => action.Kind).SequenceEqual(["propose_memory", "review", "explore", "branch", "copy"]),
            "completed assistant controls follow WebView's memory/review/explore/branch/copy order");
        Check(model.MessageActions(model.Snapshot!, 3, true).Last().Kind == "undo_preview"
            && model.MessageActions(model.Snapshot!, 3, true).All(action => action.Icon != null && action.Enabled),
            "only the latest response exposes undo and assistant controls use shared icons");
        Check(model.MessageActions(model.Snapshot!, 0).All(action => action.Icon == null), "user controls remain text actions");
        host.ExtraResponse = true; await model.RefreshAsync();
        Check(model.MessageActions(model.Snapshot!, 3).All(action => action.Kind != "undo_preview")
            && model.MessageActions(model.Snapshot!, 4).Last().Kind == "undo_preview",
            "intermediate assistant messages in the latest turn do not duplicate Undo");
        host.ExtraResponse = false; host.EmptyLatest = true; await model.RefreshAsync();
        Check(!model.MessageActions(model.Snapshot!, 3).Single(action => action.Kind == "copy").Enabled,
            "empty assistant replies cannot copy an empty payload");
        Check(model.MessageActions(model.Snapshot! with { SessionId = "foreign" }, 1).Length == 0,
            "a previous conversation's rows cannot expose current actions");
        host.EmptyLatest = false;
        host.ReadOnly = true; await model.RefreshAsync();
        Check(model.MessageActions(model.Snapshot!, 3).Where(action => action.Kind != "copy").All(action => !action.Enabled),
            "read-only history retains visible controls but disables mutations");
        host.ReadOnly = false; host.CanBranch = true; host.Acp = "agent"; await model.RefreshAsync();
        Check(model.MessageActions(model.Snapshot!, 0).Select(action => action.Kind).SequenceEqual(["copy"])
            && model.MessageActions(model.Snapshot!, 3, true).All(action => action.Kind is not ("branch" or "explore" or "undo_preview")),
            "ACP omits unsupported branch, rewind, exploration and undo controls");
        Check(!model.CanHistoryAction(target, "branch"), "ACP blocks branch even when the host's structural CanBranch flag is true");
        host.Acp = null; host.CanBranch = true; host.Reviewing = true; await model.RefreshAsync();
        Check(!model.MessageActions(model.Snapshot!, 3).Single(action => action.Kind == "review").Enabled,
            "an ongoing review disables its action without hiding the bar");
        host.Reviewing = false;
        host.Running = true; await model.RefreshAsync();
        Check(model.MessageActions(model.Snapshot!, 3, true).Length == 0
            && model.MessageActions(model.Snapshot!, 2).Select(action => action.Kind).SequenceEqual(["copy", "rewind"])
            && !model.MessageActions(model.Snapshot!, 2).Single(action => action.Kind == "rewind").Enabled,
            "streaming tail hides assistant actions and user branch while keeping copy and disabled rewind");
        Check(model.MessageActions(model.Snapshot!, 1, true).Single(action => action.Kind == "branch").Enabled
            && !model.MessageActions(model.Snapshot!, 1, true).Single(action => action.Kind == "review").Enabled
            && !model.MessageActions(model.Snapshot!, 1, true).Single(action => action.Kind == "explore").Enabled,
            "earlier response can branch during a later turn but review and exploration wait for idle");
        Check(model.CanHistoryAction(target, "branch") && model.CanHistoryAction(target, "propose_memory"), "historical actions remain available during later turns");
        Check(!model.CanHistoryAction(target, "review") && !model.CanHistoryAction(target, "undo"), "review and undo wait for the running turn");
        var latest = model.HistoryTarget(model.Snapshot!, 3)!;
        Check(!model.CanHistoryAction(latest, "branch") && !model.CanHistoryAction(latest, "propose_memory"), "current streaming turn cannot be treated as completed history");
        var branched = await model.HistoryActionAsync(target, new() { ["kind"] = "branch", ["checkpoint"] = "after_response" });
        Check(branched.Success && host.Writes.Single()["target"]?["user_index"]?.GetValue<int>() == 34, "branch sends explicit absolute checkpoint");
        Check(host.Commands.All(command => !command.Contains("stop")), "branch never stops the source turn");
        host.Running = false; await model.RefreshAsync();
        host.ReplaceOldTurn = true; await model.RefreshAsync();
        Check(!model.CanHistoryAction(target, "branch"), "stale row at reused visual index cannot branch");
        host.ReplaceOldTurn = false; await model.RefreshAsync();
        host.LoseResponse = true;
        await model.HistoryActionAsync(latest, new() { ["kind"] = "undo" });
        await model.RefreshAsync();
        Check(model.HistoryUncertain && host.Writes.Count == 2, "lost mutation response is retained without replay");
        Check(model.HistoryFailureReason == "reply lost after write", "read refresh preserves the raw failure for localized dialog guidance");
        Check(model.MessageActions(model.Snapshot!, 3).Where(action => action.Kind != "copy").All(action => !action.Enabled),
            "unconfirmed mutations immediately disable inline history controls");
        await model.HistoryActionAsync(latest, new() { ["kind"] = "undo" });
        Check(host.Writes.Count == 2, "a second mutation requires an explicit result check");
        model.AcknowledgeHistoryResult(); host.LoseResponse = false;
        Check(model.HistoryFailureReason == null, "explicit acknowledgement clears the dialog's failure detail");
        model.Draft = "my pending draft";
        await model.HistoryActionAsync(latest, new() { ["kind"] = "undo" });
        Check(model.Draft == "my pending draft", "undo does not overwrite an independent draft");
        host.Pending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var pending = model.HistoryActionAsync(target, new() { ["kind"] = "propose_memory" });
        Check(model.HistoryBusy && !model.Busy, "proposing memory does not disable the source turn's Stop control");
        Check(model.MessageActions(model.Snapshot!, 1).Where(action => action.Kind != "copy").All(action => !action.Enabled),
            "a pending memory request disables all history actions to prevent duplicate submissions");
        await model.OpenAsync("p", "other");
        host.Pending.SetResult(Fake.Reply(target));
        Check(!(await pending).Success && model.Snapshot?.SessionId == "other" && !model.Busy, "late history responses cannot show a dialog in another session");
        host.Pending = null;
        var wrong = false;
        try { await new NativeConversationHistoryClient(new ForeignReply(host)).ActAsync(target, new() { ["kind"] = "branch", ["checkpoint"] = "after_response" }, default); }
        catch (InvalidDataException) { wrong = true; }
        Check(wrong, "foreign mutation result fails ownership validation");
        Console.WriteLine("Native history: absolute targets, running actions, stale identities, no replay, draft preservation and navigation passed.");
    }
    private static void Check(bool value, string label) { if (!value) throw new InvalidOperationException(label); }
    private sealed class ForeignReply(Fake host) : INativeSettingsClient
    {
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        { var reply = await host.InvokeAsync(command, args, projectId, cancellationToken); reply!["session_id"] = "foreign"; return reply; }
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool Running, ReplaceOldTurn, LoseResponse, ReadOnly, Reviewing, ExtraResponse, EmptyLatest;
        public bool CanBranch = true;
        public string? Acp;
        public TaskCompletionSource<JsonNode?>? Pending;
        public List<JsonObject> Writes = [];
        public List<string> Commands = [];
        private ulong sequence;
        public static JsonNode Reply(NativeHistoryTarget target) => new JsonObject {
            ["session_id"] = target.SessionId, ["target"] = JsonSerializer.SerializeToNode(target.Turn, ConversationSnapshot.JsonOptions), ["result"] = "branch-id"
        };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Commands.Add(command);
            if (command == "native_conversation_snapshot")
            {
                var turns = Enumerable.Range(0, 36).Select(n => new NativeTurnIdentity(n, n == 0 ? 0 : n * 2 + 1, n == 34 && ReplaceOldTurn ? "replaced" : "digest-" + n)).ToArray();
                var items = new List<ConversationItem> {
                    new("user", "old question", null, null, null, null), new("assistant", "old answer", null, null, null, null),
                    new("user", "latest question", null, null, null, null), new("assistant", EmptyLatest ? "  " : "latest answer", null, null, null, null) };
                if (ExtraResponse) items.Add(new("assistant", "final response", null, null, null, null));
                var snapshot = new ConversationSnapshot(ConversationSnapshot.SchemaId, "e", ++sequence, "p", args["session_id"]!.GetValue<string>(),
                    items.ToArray(),
                    60, Running, false, ReadOnly, "m", null, null, [], AcpAgentId: Acp, UserOffset: 34, HistoryState: new("r", CanBranch, Reviewing, turns));
                return Task.FromResult(JsonSerializer.SerializeToNode(snapshot, ConversationSnapshot.JsonOptions));
            }
            if (command is "list_models" or "list_acp_agents") return Task.FromResult<JsonNode?>(new JsonArray());
            if (command == "native_conversation_history_action")
            {
                Check(projectId == "p", "history always sends project scope");
                Writes.Add((JsonObject)args.DeepClone());
                if (LoseResponse) throw new IOException("reply lost after write");
                if (Pending != null) return Pending.Task;
                return Task.FromResult<JsonNode?>(new JsonObject { ["session_id"] = args["session_id"]!.DeepClone(),
                    ["target"] = args["target"]!.DeepClone(), ["result"] = "branch-id" });
            }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
