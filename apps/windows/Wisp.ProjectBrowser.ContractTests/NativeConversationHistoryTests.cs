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
        host.Running = true; await model.RefreshAsync();
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
        await model.HistoryActionAsync(latest, new() { ["kind"] = "undo" });
        Check(host.Writes.Count == 2, "a second mutation requires an explicit result check");
        model.AcknowledgeHistoryResult(); host.LoseResponse = false;
        model.Draft = "my pending draft";
        await model.HistoryActionAsync(latest, new() { ["kind"] = "undo" });
        Check(model.Draft == "my pending draft", "undo does not overwrite an independent draft");
        host.Pending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var pending = model.HistoryActionAsync(target, new() { ["kind"] = "propose_memory" });
        Check(model.HistoryBusy && !model.Busy, "proposing memory does not disable the source turn's Stop control");
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
        public bool Running, ReplaceOldTurn, LoseResponse;
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
                var snapshot = new ConversationSnapshot(ConversationSnapshot.SchemaId, "e", ++sequence, "p", args["session_id"]!.GetValue<string>(),
                    [new("user", "old question", null, null, null, null), new("assistant", "old answer", null, null, null, null),
                     new("user", "latest question", null, null, null, null), new("assistant", "latest answer", null, null, null, null)],
                    60, Running, false, false, "m", null, null, [], UserOffset: 34, HistoryState: new("r", true, false, turns));
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
