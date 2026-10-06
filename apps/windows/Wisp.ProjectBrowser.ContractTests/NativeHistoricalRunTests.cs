using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeHistoricalRunTests
{
    private static void Check(bool ok, string name) { if (!ok) throw new Exception(name); Console.WriteLine("PASS historical Run: " + name); }
    public static async Task RunAsync()
    {
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s"); model.Draft = "retain draft";
        await model.OpenQuestionAsync(new(10, "old question", 100, null, null));
        var history = model.History!; var target = model.ScrollTarget; var scroll = model.ScrollRevision;
        host.Current = host.Current with { StdoutTail = "first\nsecond", ProgressJson = """{"phase":"copy","completed_bytes":75,"total_bytes":100}""" };
        await model.RefreshAsync();
        Check(model.History!.RunCards!.Single().StdoutTail == "first\nsecond" && NativeRunPresentation.Progress(model.History.RunCards.Single())?.Percent == 75,
            "historical card reads current output and byte progress even when absent from latest messages");
        Check(host.DetailCalls == 1 && host.HistoryReads == 1 && model.History.UserOffset == history.UserOffset
            && model.History.NextBeforeSeq == history.NextBeforeSeq && model.ScrollTarget == target && model.ScrollRevision == scroll
            && model.Draft == "retain draft" && model.VisibleItems[0] == history.Items[0],
            "deduplicated Run reads leave the pinned transcript, outline position and draft unchanged");
        Check(model.CanCancelRun(model.History.RunCards.Single()), "owned active historical Run remains cancellable");
        host.FailDetail = true; await model.RefreshAsync();
        Check(model.RunReadError("r") != null && model.History.RunCards.Single().StdoutTail == "first\nsecond"
            && !model.CanCancelRun(model.History.RunCards.Single()) && model.ConnectionError == null,
            "failed detail read labels retained state and disables cancellation without breaking the live conversation");
        host.FailDetail = false; host.WrongIdentity = true; await model.RefreshAsync();
        Check(model.RunReadError("r") != null && model.History.RunCards.Single().FrameId == "s", "foreign Run reply cannot replace the historical record");
        host.WrongIdentity = false; await model.RefreshAsync();
        Check(model.RunReadError("r") == null && model.CanCancelRun(model.History.RunCards.Single()), "next confirmed read recovers the card");
        host.LoseCancel = true;
        await model.CancelInlineRunAsync(model.History.RunCards.Single()); await model.CancelInlineRunAsync(model.History.RunCards.Single());
        Check(host.CancelCalls == 1 && model.RunError("r") != null, "uncertain historical cancellation is never replayed by polling or another click");
        model.AcknowledgeRunAction("r"); host.LoseCancel = false;
        await model.CancelInlineRunAsync(model.History.RunCards.Single());
        Check(host.CancelCalls == 2 && model.History.RunCards.Single().Status == "cancelling", "confirmed historical cancellation refreshes the same page");
        foreach (var status in new[] { "failed", "cancelled", "timed_out", "lost" })
        {
            host.Current = host.Current with { Status = status, EndedAt = 200, ExitCode = 1, StdoutTail = "final output" };
            await model.RefreshAsync();
            Check(model.History.RunCards.Single().Status == status && model.History.Items[1].Run?.Status == status
                && model.History.Items[2].Run?.Status == status && !model.CanCancelRun(model.History.RunCards.Single())
                && model.History.Items[1].Run?.OwnerIndex == 1,
                status + " updates every link and preserves the exact submission owner");
        }
        host.Current = host.Current with { Status = "running", EndedAt = null }; await model.RefreshAsync();
        host.Current = host.Current with { Status = "succeeded", EndedAt = 250 }; await model.RefreshAsync();
        Check(host.PromptCalls == 0 && model.RunReviewPrompt == null, "historical completion does not interrupt reading with a result browser");
        model.Latest(); await model.RefreshAsync();
        Check(host.PromptCalls == 1 && model.RunReviewPrompt?.RunId == "r" && model.Snapshot!.RunCards!.Length == 0,
            "returning to latest revalidates and nominates a completed Run whose submission is on an older page");
        Check(model.TakeRunReviewPrompt(model.RunReviewPrompt!), "deferred historical review can be consumed with exact current scope");
        await model.RefreshAsync(); Check(host.PromptCalls == 1, "deferred prompt is not reopened on the next poll");
        await CleanupAsync();
        await RacesAsync();
    }
    private static async Task CleanupAsync()
    {
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s"); await model.OlderAsync();
        host.Current = host.Current with { Status = "succeeded", EndedAt = 250 };
        await model.RefreshAsync();
        Check(model.History!.Items[1].Run?.NeedsReview == true, "unreviewed historical success exposes review actions");
        host.Current = host.Current with { CleanedAt = 300 };
        await model.RefreshAsync();
        Check(model.History.RunCards!.Single().CleanedAt == 300 && model.History.Items.Where(item => item.Run != null).All(item => !item.Run!.NeedsReview),
            "cleanup updates every historical link and removes review eligibility");
        model.DismissRunCard("foreign"); Check(!model.IsRunHidden("foreign"), "unknown historical Run cannot be dismissed");
        model.DismissRunCard("r"); Check(model.IsRunHidden("r"), "historical terminal card can be dismissed even when absent from latest page");
        model.Latest(); await model.RefreshAsync();
        Check(host.PromptCalls == 0 && model.RunReviewPrompt == null, "cleanup before returning latest suppresses the deferred review");
        await model.OpenAsync("p", "other");
        Check(!model.IsRunHidden("r"), "historical card dismissal remains scoped to its original session");
    }
    private static async Task RacesAsync()
    {
        foreach (var change in new[] { "newer_read", "older_page", "latest", "session", "pause", "cancel", "restart" })
        {
            var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
            await model.OpenAsync("p", "s"); await model.OlderAsync();
            host.Pending = new(TaskCreationOptions.RunContinuationsAsynchronously); var pending = host.Pending;
            using var cancellation = new CancellationTokenSource();
            var read = model.RefreshAsync(cancellation.Token);
            Check(host.DetailCalls == 1, change + " has a pending exact historical read");
            host.Current = host.Current with { StdoutTail = "newer output" };
            if (change == "newer_read") await model.RefreshAsync();
            if (change == "older_page") await model.OlderAsync();
            if (change == "latest") model.Latest();
            if (change == "session") await model.OpenAsync("p", "other");
            if (change == "pause") model.Pause();
            if (change == "cancel") cancellation.Cancel();
            if (change == "restart") { host.Epoch = "restarted"; await model.RefreshAsync(); }
            var expected = model.History;
            pending.SetResult(JsonSerializer.SerializeToNode(host.Current with { StdoutTail = "stale output" }, ConversationSnapshot.JsonOptions));
            await read;
            Check(ReferenceEquals(model.History, expected) && model.RunReviewPrompt == null,
                change + " discards the late historical read without changing the selected page");
        }
    }
    private sealed class Fake : INativeSettingsClient
    {
        public NativeRun Current = JsonSerializer.Deserialize<NativeRun>("""{"id":"r","frame_id":"s","context_id":"local","title":"historical run","kind":"ssh_direct","status":"running","created_at":100,"progress_json":"{}","env_snapshot_json":"{}"}""", ConversationSnapshot.JsonOptions)!;
        public int DetailCalls, HistoryReads, CancelCalls, PromptCalls;
        public bool FailDetail, WrongIdentity, LoseCancel;
        public string Epoch = "host";
        public TaskCompletionSource<JsonNode?>? Pending;
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            var session = args["session_id"]?.GetValue<string>() ?? "s";
            if (command == "native_conversation_snapshot")
            {
                var before = args["before_seq"]?.GetValue<long>();
                ConversationItem[] items = [new("user", "latest question", null, null, null, null)];
                NativeRun[] runs = [];
                if (before == 100 && session == "s")
                {
                    HistoryReads++; runs = [Current]; items = [new("user", "old question", null, null, null, null),
                        new("tool", "submission", "run_in_context", null, true, null, Run: new("r", Current.Status, 1, false)),
                        new("tool", "monitor", "monitor_run", "r", true, null, Run: new("r", Current.Status, 1, false)),
                        new("assistant", "old answer", null, null, null, null)];
                }
                var page = new ConversationSnapshot(ConversationSnapshot.SchemaId, Epoch, ++sequence, projectId!, session, items,
                    before == 100 ? 50 : before == 50 ? null : 100, false, false, false, "m", null, null, [],
                    UserOffset: before == 100 ? 10 : before == 50 ? 0 : 40, RunCards: runs, RunReviewSupported: true);
                return Task.FromResult(JsonSerializer.SerializeToNode(page, ConversationSnapshot.JsonOptions));
            }
            if (command is "list_models" or "list_acp_agents") return Task.FromResult<JsonNode?>(new JsonArray());
            if (command == "native_conversation_panel_run_detail")
            {
                if (projectId != "p" || session != "s" || args["run_id"]?.GetValue<string>() != "r") throw new Exception("Foreign historical Run read");
                DetailCalls++;
                if (Pending is { } pending) { Pending = null; return pending.Task; }
                if (FailDetail) throw new IOException("unavailable");
                return Task.FromResult(JsonSerializer.SerializeToNode(WrongIdentity ? Current with { FrameId = "foreign" } : Current, ConversationSnapshot.JsonOptions));
            }
            if (command == "native_conversation_panel_run_cancel")
            {
                CancelCalls++; if (LoseCancel) throw new IOException("response lost");
                Current = Current with { Status = "cancelling" };
                return Task.FromResult(JsonSerializer.SerializeToNode(Current, ConversationSnapshot.JsonOptions));
            }
            if (command == "native_conversation_panel_run_review")
            {
                PromptCalls++;
                return Task.FromResult(JsonSerializer.SerializeToNode(new NativeRunReviewReply("r", false, false, null, null, true, true), ConversationSnapshot.JsonOptions));
            }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
