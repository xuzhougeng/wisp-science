using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeExecutionReadingTests
{
    private static void Check(bool pass, string message) { if (!pass) throw new InvalidOperationException(message); }
    private static ConversationItem User(string text) => new("user", text, null, null, null, null);
    private static ConversationSnapshot Snapshot(string session, ConversationItem[] items, bool running = false, NativeRun[]? runs = null) =>
        new(ConversationSnapshot.SchemaId, "host", 1, "p", session, items, null, running, false, false, "m", null, null, [], UserOffset: 30, RunCards: runs, RunReviewSupported: true);
    private static NativeRun Run(string status, string session = "s") => JsonSerializer.Deserialize<NativeRun>(JsonSerializer.Serialize(new {
        id = "r", frame_id = session, context_id = "qa", title = "Synthetic run", kind = "ssh_direct", status, created_at = 100,
        progress_json = "{}", env_snapshot_json = "{}", stdout_tail = "1\r2\r3\nresult", stderr_tail = "warning"
    }), ConversationSnapshot.JsonOptions)!;
    public static async Task RunAsync()
    {
        var accepted = new ConversationItem("tool", "raw", "update_plan", null, true, null,
            PlanSteps: [new("done", "Inspect"), new("running", "Analyze\n[x] nested"), new("cancelled", "Optional")]);
        var initial = Snapshot("s", [User("turn"), accepted]);
        var plan = NativeExecutionPlan.Latest(initial)!;
        Check(plan.Done == 1 && !plan.Complete && plan.Current!.Content.Contains("nested"), "accepted plan retains factual progress and nested content");
        Check(plan.Label(false) == "计划未完成", "idle is not plan completion");
        var failed = accepted with { Ok = false, PlanSteps = [new("done", "not accepted")] };
        Check(NativeExecutionPlan.Latest(initial with { Items = [User("turn"), accepted, failed, accepted with { Ok = null }] })?.Key == plan.Key,
            "pending and rejected updates leave accepted progress intact");
        Check(NativeExecutionPlan.Latest(initial with { Items = [User("turn"), accepted, User("next")] }) == null, "a new user turn clears its predecessor plan");
        Check(NativeExecutionPlan.Latest(initial with { Items = [User("turn"), accepted, new("queued_user", "next", null, null, null, null)] })?.Key == plan.Key,
            "queued rows do not clear the current execution plan");
        Check(NativeExecutionPlan.Latest(initial with { Items = [User("turn"), accepted, accepted with { PlanSteps = [] }] }) == null, "unparseable accepted replacement cannot leave stale plan steps");
        Check(NativeExecutionPlan.Latest(initial with { SessionId = "other" })!.Key != plan.Key, "plan dismissal identity is session scoped");
        var acp = new ConversationItem("acp_tool", """[{"type":"content","content":{"type":"text","text":"literal <script>"}},{"type":"diff","path":"a.py","oldText":"a","newText":"b"},{"type":"terminal","terminalId":"terminal-1"}]""",
            "Edit file", null, true, "in_progress", CallId: "tool-1", Kind: "edit", Locations: """[{"path":"a.py","line":12}]""");
        var sections = NativeStructuredTool.Sections(acp);
        Check(sections.Length == 4 && sections[0].Text == "a.py:12" && sections[1].Text == "literal <script>" && sections[2].Text.Contains("修改后\nb"), "ACP structured sections preserve location, literal content, diff and terminal identity");
        Check(NativeToolPresentation.State(acp) == "执行中", "ACP in_progress cannot inherit completed state from a legacy ok flag");
        Check(NativeStructuredTool.Sections(acp with { Text = "not json", Locations = null }).Single().Text == "not json", "malformed ACP data remains readable as literal fallback");
        var key = NativeTranscriptRows.Keys(initial with { Items = [User("turn"), acp] })[1];
        Check(NativeTranscriptRows.Keys(initial with { Items = [User("turn"), accepted, acp] })[2] == key, "ACP identity survives nearby streaming-row insertion");
        var duplicateKeys = NativeTranscriptRows.Keys(initial with { Items = [User("turn"), acp, acp, acp with { CallId = "tool-1/occurrence:1" }] });
        Check(duplicateKeys.Distinct().Count() == 4 && duplicateKeys[1] == key, "duplicate or delimiter-bearing ACP call IDs cannot collide or replace the first row");
        var run = Run("running");
        Check(NativeRunPresentation.Output(run) == "3\nresult\n[stderr]\nwarning", "run output folds carriage-return frames and retains stderr");
        var progress = NativeRunPresentation.Progress(run with { ProgressJson = """{"phase":"upload","completed_bytes":25,"total_bytes":100,"files_completed":1,"files_total":2}""" });
        Check(progress?.Percent == 25 && progress?.Label.Contains("1/2") == true, "run progress derives from recorded transfer bytes");
        Check(NativeRunPresentation.Progress(run with { ProgressJson = """{"phase":"connecting","indeterminate":true,"completed_bytes":25,"total_bytes":100}""" })?.Percent == null, "indeterminate phases never invent completion percentages");
        foreach (var status in new[] { "succeeded", "failed", "cancelled", "timed_out", "lost" })
        {
            var owned = new ConversationItem("tool", "result", "run_in_context", null, true, null, Run: new("r", status, 1, true));
            Check(NativeToolPresentation.OwnsTerminalRun(owned, 1) && !NativeToolPresentation.InitiallyExpanded(owned, 1),
                "every terminal Run starts folded in its exact submission, independently of review nomination");
            Check(!NativeToolPresentation.OwnsTerminalRun(owned, 2), "nearby rows cannot claim terminal Run ownership");
        }
        await RunLifecycleAsync();
        await ReviewRaceAsync();
        await PlanDismissalAsync(accepted);
        await NativeHistoricalRunTests.RunAsync();
        Console.WriteLine("Native execution reading: accepted plans, ACP sections, stable identities, Run progress, deferred prompts and no-replay cancellation passed.");
    }
    private static async Task PlanDismissalAsync(ConversationItem accepted)
    {
        var done = accepted with { PlanSteps = [new("done", "Inspect")] };
        var host = new Fake { OverrideItems = [User("turn"), done] };
        var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s");
        var first = model.ExecutionPlan!;
        model.DismissExecutionPlan(first.Key);
        await model.RefreshAsync();
        Check(model.ExecutionPlan == null, "completed-plan dismissal survives snapshots");
        await model.OpenAsync("p", "other"); await model.OpenAsync("p", "s");
        Check(model.ExecutionPlan == null, "completed-plan dismissal survives navigation in the model");
        host.OverrideItems = [User("turn"), done, User("next"), done]; await model.RefreshAsync();
        Check(model.ExecutionPlan is { Complete: true } next && next.Key != first.Key,
            "an identical plan in a new actual turn is not suppressed by a previous dismissal");
        host.OverrideItems = [User("turn"), accepted]; await model.RefreshAsync();
        model.DismissExecutionPlan(model.ExecutionPlan!.Key);
        Check(model.ExecutionPlan != null, "incomplete plans cannot be dismissed as complete");
    }
    private static async Task ReviewRaceAsync()
    {
        foreach (var scenario in new[] { "cleaned", "foreign", "readonly", "failure", "running", "navigation" })
        {
            var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
            await model.OpenAsync("p", "s");
            host.CurrentRun = host.CurrentRun with { Status = "succeeded", EndedAt = 200 };
            host.PendingPrompt = new(TaskCreationOptions.RunContinuationsAsynchronously);
            await model.RefreshAsync();
            Check(host.PromptCalls == 1, "one pending review check per Run");
            if (scenario == "cleaned") host.CurrentRun = host.CurrentRun with { CleanedAt = 201 };
            if (scenario == "running") host.Running = true;
            if (scenario == "navigation") await model.OpenAsync("p", "other"); else await model.RefreshAsync();
            var delivered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            void Changed() => delivered.TrySetResult();
            model.Changed += Changed;
            if (scenario == "failure") host.PendingPrompt.SetException(new IOException("check failed"));
            else host.PendingPrompt.SetResult(JsonSerializer.SerializeToNode(new NativeRunReviewReply(
                scenario == "foreign" ? "foreign" : "r", scenario == "readonly", false, null, null, true, true), ConversationSnapshot.JsonOptions));
            // A discarded response deliberately sends no notification after navigation.
            if (scenario != "navigation") await delivered.Task.WaitAsync(TimeSpan.FromSeconds(5));
            model.Changed -= Changed;
            Check(model.RunReviewPrompt == null, scenario + " response must not open a review prompt");
            if (scenario is "foreign" or "failure")
            {
                await model.RefreshAsync();
                Check(host.PromptCalls == 1 && model.OperationError != null, "automatic check failure does not cause polling storms");
            }
            if (scenario == "running")
            {
                host.PendingPrompt = null; host.Running = false; await model.RefreshAsync();
                Check(model.RunReviewPrompt != null, "prompt remains eligible when the owning conversation becomes idle again");
                var prompt = model.RunReviewPrompt!;
                await model.OpenAsync("p", "other");
                Check(!model.TakeRunReviewPrompt(prompt) && model.OperationError == null && model.RunReviewPrompt == null && host.DismissCalls == 0,
                    "navigation invalidates the old nomination without dismissing an unopened result browser");
            }
        }
    }
    private static async Task RunLifecycleAsync()
    {
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s");
        var running = host.CurrentRun;
        host.LoseCancel = true;
        await model.CancelInlineRunAsync(running); await model.CancelInlineRunAsync(running);
        Check(host.CancelCalls == 1 && model.RunError("r") != null && !model.Busy, "uncertain Run cancellation never replays or disables composer Stop");
        model.AcknowledgeRunAction("r"); host.LoseCancel = false;
        host.CurrentRun = running with { Status = "succeeded", EndedAt = 200 }; host.Running = true;
        await model.RefreshAsync();
        Check(host.PromptCalls == 0 && model.RunReviewPrompt == null, "completed run defers its product prompt while owning conversation is running");
        host.Running = false; await model.RefreshAsync();
        Check(host.PromptCalls == 1 && model.RunReviewPrompt?.RunId == "r", "root model nominates completed Run independently of mounted card");
        var prompt = model.RunReviewPrompt!;
        Check(!model.TakeRunReviewPrompt(prompt with { SessionId = "foreign" }), "a prompt cannot be consumed by another session");
        Check(model.TakeRunReviewPrompt(prompt) && !model.TakeRunReviewPrompt(prompt), "direct results browser consumes its nomination exactly once");
        await model.RefreshAsync();
        Check(host.DismissCalls == 0 && model.RunReviewPrompt == null && host.PromptCalls == 1, "opening is not a dismissal and polling does not reopen the results browser");
        host.CurrentRun = running; await model.RefreshAsync(); host.CurrentRun = running with { Status = "succeeded", EndedAt = 201 };
        host.PendingPrompt = new(TaskCreationOptions.RunContinuationsAsynchronously);
        await model.RefreshAsync(); await model.OpenAsync("p", "other");
        host.PendingPrompt.SetResult(JsonSerializer.SerializeToNode(new NativeRunReviewReply("r", false, false, null, null, true, true), ConversationSnapshot.JsonOptions));
        await Task.Yield();
        Check(model.Snapshot?.SessionId == "other" && model.RunReviewPrompt == null, "late prompt cannot open review in a different session");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public NativeRun CurrentRun = Run("running");
        public bool Running, LoseCancel;
        public int PromptCalls, DismissCalls, CancelCalls;
        public TaskCompletionSource<JsonNode?>? PendingPrompt;
        public ConversationItem[]? OverrideItems;
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            var session = args["session_id"]?.GetValue<string>() ?? "s";
            if (command == "native_conversation_snapshot")
            {
                var items = session == "s" ? new[] { User("run it"), new ConversationItem("tool", "output", "monitor_run", "r", true, null, Run: new("r", CurrentRun.Status, null, false)) } : [];
                if (session == "s" && OverrideItems != null) items = OverrideItems;
                return Task.FromResult(JsonSerializer.SerializeToNode(Snapshot(session, items, Running, session == "s" ? [CurrentRun] : []) with { Sequence = ++sequence }, ConversationSnapshot.JsonOptions));
            }
            if (command is "list_models" or "list_acp_agents") return Task.FromResult<JsonNode?>(new JsonArray());
            if (command == "native_conversation_panel_run_detail") return Task.FromResult(JsonSerializer.SerializeToNode(CurrentRun, ConversationSnapshot.JsonOptions));
            if (command == "native_conversation_panel_run_cancel") { CancelCalls++; if (LoseCancel) throw new IOException("lost"); return Task.FromResult(JsonSerializer.SerializeToNode(CurrentRun, ConversationSnapshot.JsonOptions)); }
            if (command == "native_conversation_panel_run_review")
            {
                Check(projectId == "p" && session == "s" && args["run_id"]?.GetValue<string>() == "r", "review checks carry exact scope");
                if (args["operation"]?["action"]?.GetValue<string>() == "check_prompt") { PromptCalls++; if (PendingPrompt != null) return PendingPrompt.Task; }
                else DismissCalls++;
                return Task.FromResult(JsonSerializer.SerializeToNode(new NativeRunReviewReply("r", false, false, null, null, true, true), ConversationSnapshot.JsonOptions));
            }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
