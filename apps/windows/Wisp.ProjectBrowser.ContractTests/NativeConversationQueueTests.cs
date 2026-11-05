using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeConversationQueueTests
{
    public static async Task RunAsync()
    {
        Check(NativeQueueSnapshot.RequestQueueId(Guid.Parse("00010203-0405-0607-0809-0a0b0c0d0e0f")) == "506097522914230528", "UUID queue mapping matches Rust network-order bytes");
        Check(NativeQueueSnapshot.ValidId("18446744073709551615") && !NativeQueueSnapshot.ValidId("01"), "queue IDs retain all 64 bits");
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s");
        model.Draft = "  "; Check(model.ShowStopButton && !model.CanSubmit, "running with a blank draft shows only Stop");
        Check(await model.AttachAsync("a.csv") && !model.ShowStopButton && model.CanSubmit && !model.CanSend,
            "an attachment without text replaces Stop with an enabled queue action, never an immediate send");
        model.Draft = "same"; await model.QueueAsync();
        Check(model.ShowStopButton && !model.CanSubmit, "queue acknowledgement consumes the draft and brings Stop back");
        model.Draft = "same"; Check(await model.AttachAsync("b.csv"), "second attachment accepted"); await model.QueueAsync();
        Check(model.QueuedTurns.Length == 2 && host.EnqueueCalls == 2 && model.QueuedTurns[0].Id != model.QueuedTurns[1].Id, "identical text with distinct files remains two authoritative queued items");
        Check(model.QueuedTurns[0].Attachments.Single() == "uploads/a.csv" && model.QueuedTurns[1].Attachments.Single() == "uploads/b.csv", "queue snapshot retains each attachment payload");
        var target = model.QueueTarget(model.QueuedTurns[0])!;
        host.Rows["s"][0] = host.Rows["s"][0] with { Digest = "edited-elsewhere" };
        await model.RefreshAsync();
        Check(!model.CanQueueAction(target, "cancel"), "stale queue payload cannot cancel another edit");
        target = model.QueueTarget(model.QueuedTurns[1])!;
        Check(model.CanQueueAction(target, "move_up") && !model.CanQueueAction(target, "move_down"), "sorting availability follows authoritative order");
        await model.QueueActionAsync(target, new() { ["kind"] = "move_up" });
        Check(model.QueuedTurns[0].Id == target.Item.Id && host.Actions.Single()["id"]!.GetValue<string>() == target.Item.Id, "move identifies a row by stable ID");
        host.CutIn = false; await model.RefreshAsync();
        Check(!model.CanQueueAction(target, "cut_in"), "ACP and timer cut-ins remain unavailable");
        host.CutIn = true; host.LoseAction = true;
        await model.QueueActionAsync(target, new() { ["kind"] = "replace" });
        await model.RefreshAsync(); await model.QueueActionAsync(target, new() { ["kind"] = "replace" });
        Check(model.QueueUncertain && host.Actions.Count == 2, "ambiguous replace is not replayed by poll or a second action");
        model.AcknowledgeUncertainSend(); host.LoseAction = false;
        host.PendingAction = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var pending = model.QueueActionAsync(target, new() { ["kind"] = "cancel" });
        await model.OpenAsync("p", "other"); model.Draft = "other draft";
        host.PendingAction.SetResult(new JsonObject { ["session_id"] = "s", ["id"] = target.Item.Id });
        Check(!await pending && model.Snapshot!.SessionId == "other" && model.Draft == "other draft" && !model.Busy, "late queue responses cannot change the selected session");
        host.PendingAction = null;
        await model.OpenAsync("p", "s"); model.Draft = "";
        host.Epoch = "restart"; host.Rows["s"].Clear(); await model.RefreshAsync();
        Check(model.QueueRecoveries.Length == 2, "host restart retains both unconfirmed queued payloads for explicit recovery");
        model.RestoreQueueDraft(model.QueueRecoveries[0].Id);
        Check(model.Draft == "same" && model.Attachments.Length == 1 && host.EnqueueCalls == 2, "recovery restores a draft and never resends it");
        await LostEnqueueAsync();
        await EnqueueNavigationAsync();
        await ReturnDraftAsync();
        Presentation();
        Console.WriteLine("Native queue: exact IDs, multi-item payloads, stale actions, cut-in restrictions, no replay, restart recovery and navigation passed.");
    }
    private static void Presentation()
    {
        var mixed = NativeQueuePresentation.Mix((255, 255, 255, 255), (30, 13, 148, 136), .82);
        Check(mixed.A is 214 or 215 && mixed.R >= 248 && mixed.G >= 252, "CSS color mixing preserves premultiplied alpha instead of making translucent accents opaque");
        Check(NativeQueuePresentation.Mix((0, 255, 0, 0), (0, 0, 255, 0), .5) == (0, 0, 0, 0), "transparent mix does not divide by zero");
        var row = new ConversationItem("queued_user", "<script>literal user text</script>", null, null, null, null);
        var snapshot = new ConversationSnapshot(ConversationSnapshot.SchemaId, "epoch", 1, "p", "s", [row, row], null, true, false, false, "", null, null, []);
        Check(NativeTranscriptRows.RendersNothing(row) && !NativeTranscriptRows.RendersNothing(row with { Role = "user" }), "queued messages live only above the composer without hiding accepted user turns");
        var legacy = NativeQueuePresentation.Rows(snapshot, "same legacy acknowledgement");
        Check(legacy.Length == 2 && legacy.All(item => item.Item == null && item.Text == row.Text), "fallback preserves duplicate user text and never invents actionable IDs");
        Check(NativeQueuePresentation.Rows(snapshot with { Items = [] }, "legacy").Single().Text == "legacy", "acknowledged old-host queue remains visible");
        Check(NativeQueuePresentation.Rows(snapshot with { Queue = new([], [], true) }, "stale").Length == 0, "authoritative empty queue suppresses stale transcript and legacy data");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>(), "en");
        Check(NativeQueuePresentation.RecoveryReason("returned", locale).StartsWith("Returned"), "recovery copy follows selected language");
        locale.Preview("zh"); Check(NativeQueuePresentation.RecoveryReason("failed", locale).StartsWith("排队执行失败"), "failure recovery has Chinese copy");
    }
    private static async Task ReturnDraftAsync()
    {
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s");
        host.Rows["s"].Add(new("7", "authoritative-edit", "queued", "latest text\n\nUploaded files: uploads/a.csv", ["uploads/a.csv"], [new("session", "source")]));
        await model.RefreshAsync(); var target = model.QueueTarget(model.QueuedTurns.Single())!;
        model.Draft = "independent draft";
        Check(await model.TakeQueuedDraftAsync(target), "editing first acknowledges removing the queued item");
        model.RestoreQueueDraft("7");
        Check(model.Draft == "independent draft" && model.QueueRecoveries.Single().Reason == "returned" && model.QueuedTurns.Length == 0, "taking a queued message preserves an occupied composer and retains a recovery");
        await model.RefreshAsync(); model.Draft = ""; model.RestoreQueueDraft("7");
        Check(model.Draft == "latest text" && model.Attachments.Single().Path == "uploads/a.csv" && model.References.Single().Reference.Id == "source", "restoring uses the authoritative edited text, attachments and references");
        Check(host.Actions.Single()["action"]?["kind"]?.GetValue<string>() == "cancel", "Edit removes from the queue rather than changing an executing turn in place");
        host.Rows["s"].Add(new("8", "next", "queued", "for later", [], [])); await model.RefreshAsync(); target = model.QueueTarget(model.QueuedTurns.Single())!;
        host.PendingAction = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var pending = model.TakeQueuedDraftAsync(target); await model.OpenAsync("p", "other"); model.Draft = "other draft";
        host.PendingAction.SetResult(new JsonObject { ["session_id"] = "s", ["id"] = "8" });
        Check(!await pending && model.Draft == "other draft", "late acknowledgement cannot replace another session draft or dispatch side chat");
        host.PendingAction = null; host.Rows["s"].Clear(); await model.OpenAsync("p", "s");
        Check(model.QueueRecoveries.Any(item => item.Id == "8" && item.Text == "for later"), "confirmed late cancellation keeps its payload in the original session");
        host.Rows["s"].Add(new("9", "fail", "queued", "uncertain removal", [], [])); await model.RefreshAsync(); host.LoseAction = true;
        Check(!await model.TakeQueuedDraftAsync(model.QueueTarget(model.QueuedTurns.Single())!) && model.QueueRecoveries.All(item => item.Id != "9")
            && model.QueueErrorDetail == "lost action response", "uncertain cancellation cannot restore or send a message that may still be queued and retains untranslated transport details");
    }
    private static async Task EnqueueNavigationAsync()
    {
        var host = new Fake { PendingEnqueue = new(TaskCreationOptions.RunContinuationsAsynchronously) };
        var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s"); model.Draft = "pending";
        Check(await model.AttachAsync("old.csv"), "pending attachment accepted");
        var pending = model.QueueAsync();
        await model.OpenAsync("p", "other"); model.Draft = "other text";
        Check(await model.AttachAsync("other.csv"), "independent attachment accepted");
        await model.OpenAsync("p", "s"); // The snapshot confirms the enqueue before its reply arrives.
        model.Draft = "new draft";
        Check(await model.AttachAsync("new.csv"), "confirmed enqueue permits a new attachment");
        host.Rows["s"].Clear(); host.Outcomes["s"].Add(new(host.LastEnqueued!.Id, "completed"));
        await model.RefreshAsync();
        host.PendingEnqueue.SetException(new IOException("reply lost after completion")); await pending;
        Check(!model.QueueUncertain && model.Draft == "new draft" && model.Attachments.Single().Path == "uploads/new.csv", "late lost reply cannot undo authoritative confirmation or consume a new draft");
        host.Epoch = "restarted"; host.Outcomes["s"].Clear(); await model.RefreshAsync();
        Check(model.QueueRecoveries.Length == 0, "completed enqueue is never recovered after a late reply or host restart");
        await model.OpenAsync("p", "other");
        Check(model.Draft == "other text" && model.Attachments.Single().Path == "uploads/other.csv", "enqueue reconciliation preserves the independent session payload");
    }
    private static async Task LostEnqueueAsync()
    {
        var host = new Fake { LoseEnqueue = true }; var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s"); model.Draft = "uncertain";
        await model.QueueAsync(); await model.QueueAsync();
        Check(model.QueueUncertain && model.Draft == "uncertain" && host.EnqueueCalls == 1, "unknown enqueue keeps its draft and cannot replay");
        model.Draft = "new unsent text"; host.Rows["s"].Add(host.LastEnqueued!); await model.RefreshAsync();
        Check(!model.QueueUncertain && model.Draft == "new unsent text" && host.EnqueueCalls == 1, "authoritative matching ID resolves uncertainty without consuming a new draft");
        host.Rows["s"].Clear(); host.Outcomes["s"].Add(new(host.LastEnqueued!.Id, "started")); await model.RefreshAsync();
        host.Outcomes["s"] = [new(host.LastEnqueued.Id, "failed")]; await model.RefreshAsync();
        Check(model.QueueRecoveries.Single().Text == "uncertain" && !model.CanRecoverQueue, "failed queued dispatch retains the original payload without replacing the current draft");
        model.Draft = ""; model.RestoreQueueDraft(host.LastEnqueued.Id);
        Check(model.Draft == "uncertain", "explicit recovery can restore a failed dispatch");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public Dictionary<string, List<NativeQueueItem>> Rows = [];
        public Dictionary<string, List<NativeQueueOutcome>> Outcomes = [];
        public List<JsonObject> Actions = [];
        public int EnqueueCalls;
        public bool CutIn = true, LoseAction, LoseEnqueue;
        public string Epoch = "host";
        public NativeQueueItem? LastEnqueued;
        public TaskCompletionSource<JsonNode?>? PendingAction;
        public TaskCompletionSource<JsonNode?>? PendingEnqueue;
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            var session = args["session_id"]?.GetValue<string>() ?? "s";
            Rows.TryAdd(session, []); Outcomes.TryAdd(session, []);
            if (command == "native_conversation_snapshot") return Task.FromResult(JsonSerializer.SerializeToNode(new ConversationSnapshot(
                ConversationSnapshot.SchemaId, Epoch, ++sequence, "p", session, [], null, true, false, false, "m", null, null, [],
                ComposerReferences: true, Queue: new(Rows[session].ToArray(), Outcomes[session].ToArray(), CutIn)), ConversationSnapshot.JsonOptions));
            if (command is "list_models" or "list_acp_agents") return Task.FromResult<JsonNode?>(new JsonArray());
            if (command == "native_conversation_attach") return Task.FromResult<JsonNode?>(new JsonObject { ["path"] = "uploads/" + args["path"]!.GetValue<string>(), ["name"] = args["path"]!.DeepClone() });
            if (command == "native_conversation_enqueue")
            {
                EnqueueCalls++;
                LastEnqueued = new(NativeQueueSnapshot.RequestQueueId(Guid.Parse(args["request_id"]!.GetValue<string>())), "payload-" + EnqueueCalls,
                    "queued", args["message"]!.GetValue<string>(), args["attachments"]!.Deserialize<string[]>()!, []);
                if (LoseEnqueue) throw new IOException("lost enqueue response");
                Rows[session].Add(LastEnqueued);
                if (PendingEnqueue != null) return PendingEnqueue.Task;
                return Task.FromResult<JsonNode?>(new JsonObject { ["queued"] = true });
            }
            if (command == "native_conversation_queue_action")
            {
                Check(projectId == "p", "queue actions require a project scope"); Actions.Add((JsonObject)args.DeepClone());
                if (LoseAction) throw new IOException("lost action response");
                if (PendingAction != null) return PendingAction.Task;
                var id = args["id"]!.GetValue<string>(); var index = Rows[session].FindIndex(row => row.Id == id);
                if (args["action"]?["kind"]?.GetValue<string>() == "move_up") (Rows[session][index - 1], Rows[session][index]) = (Rows[session][index], Rows[session][index - 1]);
                if (args["action"]?["kind"]?.GetValue<string>() == "cancel") { Rows[session].RemoveAt(index); Outcomes[session].Add(new(id, "cancelled")); }
                return Task.FromResult<JsonNode?>(new JsonObject { ["session_id"] = session, ["id"] = id });
            }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
