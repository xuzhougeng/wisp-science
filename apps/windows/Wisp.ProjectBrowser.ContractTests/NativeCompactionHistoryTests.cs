using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeCompactionHistoryTests
{
    private static void Check(bool pass, string message) { if (!pass) throw new InvalidOperationException(message); }
    private static ConversationItem Item(string role, string text) => new(role, text, null, null, null, null);
    private static NativeCompactionRecord Record(WorkspaceConversationModel model) => NativeCompactionRecord.Parse(model.Snapshot!.Items.Last())!;
    public static async Task RunAsync()
    {
        Check(NativeCompactionRecord.Parse(Item("assistant", "{}")) == null
            && NativeCompactionRecord.Parse(Item("compaction", "broken")) == null
            && NativeCompactionRecord.Parse(Item("compaction", "{\"before\":\"1\",\"after\":0}")) == null, "Malformed or unrelated rows remain ordinary transcript rows.");
        var legacy = NativeCompactionRecord.Parse(Item("compaction", "{\"before\":0,\"after\":0,\"epoch\":null,\"kept_from_user_index\":null}"))!;
        Check(legacy is { Epoch: null, KeptFromUserIndex: null, CanUndo: false, Reduction: null }
            && legacy.Counts == "0 → 0 tokens", "Optional null fields are valid legacy records and zero never divides by zero.");
        Check((legacy with { Before = 2000, After = 990 }).Reduction == "51"
            && (legacy with { Before = 10000, After = 2000 }).Counts == "10.0k → 2.0k tokens", "Counts and reduction match WebView formatting.");
        Check((legacy with { Strategy = "auto_continue" }).AutomaticContinue
            && (legacy with { Undone = true }).TitleKey == "chat.compaction_undone"
            && (legacy with { UndoReason = "future" }).UndoReasonKey == "chat.compaction_undo_reason_has_new_turns", "Automatic continuation, undone and unknown reasons have explicit presentation.");
        var fake = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(fake), fake);
        await model.OpenAsync("p", "s"); model.Draft = "Keep this draft";
        var record = Record(model); var target = model.CompactionTarget(model.Snapshot!, record)!;
        Check(model.CanUndoCompaction(target) && fake.Undos == 0, "An eligible snapshot enables undo without eager mutation.");
        var rewind = model.CompactionRewindTarget(model.Snapshot!, record)!;
        Check(rewind.Turn.UserIndex == 1 && rewind.Turn.UserSeq == 3 && rewind.Draft == "second" && rewind.Revision == "r",
            "Rewind selects the exact retained global user turn with its durable revision.");
        var paged = model.Snapshot! with { UserOffset = 1, Items = [Item("user", "checkpoint") with { Kind = "checkpoint" }, Item("assistant", "summary"), Item("user", "second")] };
        Check(NativeCompactionRecord.RewindRow(paged, 1) == 2 && NativeCompactionRecord.RewindRow(paged, 0) == null
            && NativeCompactionRecord.RewindRow(paged with { UserOffset = null }, 1) == null
            && model.CompactionRewindTarget(paged, record)?.Turn.UserIndex == 1, "Paging offsets exclude checkpoints; missing turns cannot guess a rewind target.");
        fake.ReadOnly = true; await model.RefreshAsync(); Check(!model.CanUndoCompaction(target) && model.CompactionRewindTarget(model.Snapshot!, record) == null, "Read-only rows cannot mutate.");
        fake.ReadOnly = false; fake.Running = true; await model.RefreshAsync(); Check(!model.CanUndoCompaction(target), "Running conversations cannot undo.");
        fake.Running = false; fake.Acp = "agent"; await model.RefreshAsync(); Check(!model.CanUndoCompaction(target), "ACP cannot undo built-in context.");
        fake.Acp = null; fake.Reviewing = true; await model.RefreshAsync(); Check(!model.CanUndoCompaction(target), "Review in progress cannot undo.");
        fake.Reviewing = false; await model.RefreshAsync();
        Check(!model.CanUndoCompaction(target with { HostEpoch = "stale" }) && !model.CanUndoCompaction(target with { Epoch = 1 }), "Foreign host epochs and missing context epochs cannot mutate.");
        fake.Fail = true; await model.UndoCompactionAsync(target);
        Check(fake.Undos == 1 && model.CompactionUndoOutcome(target)?.Error != null && !model.CanUndoCompaction(target), "Unknown outcome remains visible and disables replay.");
        await model.RefreshAsync(); await model.UndoCompactionAsync(target); Check(fake.Undos == 1, "Polling and repeated calls never replay an unconfirmed operation.");
        fake.FailRead = true; await model.ReloadCompactionUndoAsync(target); Check(model.CompactionUndoOutcome(target)?.Error != null, "Failed explicit reload cannot clear uncertainty.");
        fake.FailRead = false; await model.ReloadCompactionUndoAsync(target); Check(model.CanUndoCompaction(target), "Explicit successful fresh reload permits a new deliberate attempt.");
        fake.Fail = false; await model.UndoCompactionAsync(target);
        Check(fake.Undos == 2 && model.CompactionUndoOutcome(target)?.Confirmed == true && !model.CanUndoCompaction(target)
            && model.CompactionRewindTarget(model.Snapshot!, record) == null && model.Draft == "Keep this draft" && !model.Busy && !model.HistoryBusy,
            "Confirmed undo prevents replay and rewind without consuming the draft.");
        Check(fake.LastArgs?.Count == 2 && fake.LastArgs["session_id"]?.GetValue<string>() == "s" && fake.LastArgs["head_epoch"]?.GetValue<ulong>() == 2,
            "Undo uses only the existing scoped session and expected-head contract.");

        fake = new Fake { WrongReply = true }; model = new(new NativeConversationClient(fake), fake); await model.OpenAsync("p", "s");
        target = model.CompactionTarget(model.Snapshot!, Record(model))!; await model.UndoCompactionAsync(target);
        Check(model.CompactionUndoOutcome(target)?.Error != null, "A response with the wrong undone epoch is not accepted.");
        fake.WrongReply = false; await model.ReloadCompactionUndoAsync(target); fake.Pending = new();
        var pending = model.UndoCompactionAsync(target); Check(model.Busy && model.HistoryBusy, "Pending undo locks duplicate submissions.");
        await model.OpenAsync("p", "other"); await model.OpenAsync("p", "s");
        Check(!model.CanUndoCompaction(target), "Navigating away and back preserves the in-flight undo guard.");
        await model.UndoCompactionAsync(target); Check(fake.Undos == 2, "Returning to the original session cannot submit an in-flight undo twice.");
        await model.OpenAsync("p", "other"); model.Draft = "Other draft"; fake.Pending.SetResult(Fake.Reply("s", 2)); await pending;
        Check(model.CurrentSessionId == "other" && model.Draft == "Other draft" && !model.Busy && !model.HistoryBusy,
            "Late replies cannot alter the active session or its draft.");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool Running, ReadOnly, Reviewing, Fail, FailRead, WrongReply; public string? Acp;
        public int Undos; public JsonObject? LastArgs; public TaskCompletionSource<JsonNode?>? Pending; private ulong sequence;
        public static JsonNode Reply(string session, ulong epoch) => new JsonObject { ["project_id"] = "p", ["session_id"] = session, ["undone_epoch"] = epoch };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "native_conversation_snapshot")
            {
                if (FailRead) throw new IOException("Synthetic read failure");
                return Task.FromResult(JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", ++sequence, projectId!, args["session_id"]!.GetValue<string>(),
                    [Item("user", "first"), Item("assistant", "answer"), Item("user", "second"), Item("compaction", "{\"before\":10000,\"after\":2000,\"strategy\":\"manual\",\"epoch\":2,\"checkpoint\":\"## Summary\",\"kept_from_user_index\":1,\"can_undo\":true}")],
                    null, Running, false, ReadOnly, "m", null, null, [], AcpAgentId: Acp, UserOffset: 0,
                    HistoryState: new("r", true, Reviewing, [new(0, 1, "first"), new(1, 3, "second")])), ConversationSnapshot.JsonOptions));
            }
            if (command is "list_models" or "list_acp_agents") return Task.FromResult<JsonNode?>(new JsonArray());
            if (command == "native_conversation_context_undo")
            {
                Check(projectId == "p", "Undo always includes project scope."); Undos++; LastArgs = (JsonObject)args.DeepClone();
                if (Fail) throw new IOException("Synthetic connection loss");
                return Pending?.Task ?? Task.FromResult<JsonNode?>(Reply(args["session_id"]!.GetValue<string>(), WrongReply ? 99u : 2u));
            }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
