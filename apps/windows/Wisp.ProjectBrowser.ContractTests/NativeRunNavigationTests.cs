using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeRunNavigationTests
{
    public static async Task RunAsync()
    {
        await EnvironmentReadsAsync();
        var transport = new Fake();
        var model = Create(transport);
        await model.OpenRunAsync("run-a");
        Check(model.Tabs.Selected == "runs" && model.SelectedRunId == "run-a" && model.RunDetail?.Id == "run-a" && !model.RunLoading,
            "message navigation opens the Runs tab and the exact run");
        Check(transport.Calls.All(call => call.Project == "project-a" && call.Args["session_id"]!.GetValue<string>() == "session-a"),
            "every read explicitly scopes project and session");
        Check(transport.Calls.All(call => call.Command is "native_conversation_panel_contexts" or "native_conversation_panel_activity" or "native_conversation_panel_run_detail"),
            "opening run details never starts a runtime or mutates a run");

        var slowTransport = new Fake();
        var slowModel = Create(slowTransport);
        var contexts = new TaskCompletionSource<JsonNode?>();
        var contextReads = 0;
        slowTransport.Handler = (command, args) => command == "native_conversation_panel_activity" && ++contextReads == 1
            ? contexts.Task : Task.FromResult(slowTransport.Default(command, args));
        var oldOpen = slowModel.OpenRunAsync("first");
        var newOpen = slowModel.OpenRunAsync("second");
        contexts.SetResult(slowTransport.Default("native_conversation_panel_activity", new())); await Task.WhenAll(oldOpen, newOpen);
        Check(slowModel.RunDetail?.Id == "second" && slowTransport.Calls.Count(call => call.Command == "native_conversation_panel_run_detail") == 1,
            "superseded open stops before its detail read even when activity loading was delayed");

        var first = new TaskCompletionSource<JsonNode?>();
        var second = new TaskCompletionSource<JsonNode?>();
        transport.Handler = (command, args) => command == "native_conversation_panel_run_detail"
            ? args["run_id"]!.GetValue<string>() == "first" ? first.Task : second.Task
            : Task.FromResult(transport.Default(command, args));
        var older = model.ReadRunAsync("first");
        Check(model.RunDetail == null && model.RunLoading, "selecting another run immediately removes the previous output");
        var newer = model.ReadRunAsync("second");
        second.SetResult(Run("second")); await newer;
        first.SetResult(Run("first")); await older;
        Check(model.SelectedRunId == "second" && model.RunDetail?.Id == "second" && !model.RunLoading, "late first selection cannot overwrite the second run");

        var delayed = new TaskCompletionSource<JsonNode?>();
        transport.Handler = (command, args) => command == "native_conversation_panel_run_detail" ? delayed.Task : Task.FromResult(transport.Default(command, args));
        var read = model.ReadRunAsync("second");
        model.DismissRun(); delayed.SetResult(Run("second")); await read;
        Check(model.RunDetail == null && model.SelectedRunId == null && !model.RunLoading, "closing inline details invalidates an in-flight read");

        delayed = new(); read = model.ReadRunAsync("third");
        await model.RefreshAsync("files");
        delayed.SetException(new IOException("late read error")); await read;
        Check(model.Tabs.Selected == "files" && model.RunDetail == null && model.RunError == null && !model.RunLoading, "late errors cannot reopen details after leaving Runs");

        transport.Handler = (command, args) => Task.FromResult(command == "native_conversation_panel_run_detail" ? Run("wrong-id") : transport.Default(command, args));
        await model.OpenRunAsync("run-a");
        Check(model.RunDetail == null && model.RunError?.Contains("身份") == true, "mismatched detail identity is rejected without replay");
        transport.Handler = null; await model.ReadRunAsync("run-a");
        transport.Handler = (command, args) => command == "native_conversation_panel_run_detail" ? throw new IOException("offline") : Task.FromResult(transport.Default(command, args));
        await model.ReadRunAsync("run-a");
        Check(model.RunDetail?.Id == "run-a" && model.RunError == "offline" && !model.RunLoading, "same-run refresh failure preserves the last snapshot with a visible error");
        transport.Handler = (command, args) => command == "native_conversation_panel_run_detail" ? throw new OperationCanceledException() : Task.FromResult(transport.Default(command, args));
        await model.ReadRunAsync("run-a");
        Check(!model.RunLoading && model.RunError?.Contains("取消") == true, "cancelled read releases loading and reports its outcome");
        var calls = transport.Calls.Count;
        await model.ReadRunAsync("run-a", new CancellationToken(true));
        Check(transport.Calls.Count == calls, "already-cancelled navigation starts no read");

        transport.Handler = null; await model.RefreshAsync("runs");
        await model.CancelRunAsync("run-a");
        Check(!model.ActivityBusy && transport.Calls.Count(call => call.Command == "native_conversation_panel_run_cancel") == 1,
            "successful mutation releases busy after its own generation-changing refresh");
        var write = new TaskCompletionSource<JsonNode?>();
        transport.Handler = (command, args) => command == "native_conversation_panel_run_cancel" ? write.Task : Task.FromResult(transport.Default(command, args));
        var cancel = model.CancelRunAsync("run-a");
        await model.RefreshAsync("files");
        write.SetResult(Run("run-a")); await cancel;
        Check(!model.ActivityBusy && model.Tabs.Selected == "files", "late mutation does not reopen Runs or leave busy stuck");
        transport.Handler = null; await model.RefreshAsync("runs");
        transport.Handler = (command, args) => Task.FromResult(command == "native_conversation_panel_run_cancel" ? Run("wrong-id") : transport.Default(command, args));
        var mutations = transport.Calls.Count(call => call.Command == "native_conversation_panel_run_cancel");
        await model.CancelRunAsync("run-a");
        Check(!model.ActivityBusy && model.Error?.Contains("身份") == true
            && transport.Calls.Count(call => call.Command == "native_conversation_panel_run_cancel") == mutations + 1,
            "mismatched mutation result is uncertain and never automatically replayed");
        transport.Handler = (command, args) => command == "native_conversation_panel_run_cancel" ? throw new OperationCanceledException() : Task.FromResult(transport.Default(command, args));
        await model.CancelRunAsync("run-a");
        Check(!model.ActivityBusy && model.Error?.Contains("未确认") == true, "cancelled mutation releases busy and reports uncertainty");
        transport.Handler = null; transport.ReadOnly = true; await model.RefreshAsync("runs");
        calls = transport.Calls.Count; await model.CancelRunAsync("run-a"); await model.HarvestRunAsync("run-a");
        Check(transport.Calls.Count == calls, "read-only run panels reject mutations in the model");
        transport.ReadOnly = false; await model.RefreshAsync("runs");
        delayed = new(); transport.Handler = (_, _) => delayed.Task;
        read = model.ReadRunAsync("run-a"); model.Close(); delayed.SetResult(Run("run-a")); await read;
        Check(model.RunDetail == null && model.SelectedRunId == null && !model.RunLoading, "disposed panel rejects late details");
        transport.Handler = null;
        var fresh = Create(transport);
        transport.Handler = (command, args) => command == "native_conversation_panel_contexts" ? throw new IOException("context discovery unavailable") : Task.FromResult(transport.Default(command, args));
        await fresh.RefreshAsync("runs");
        Check(fresh.Error == null && fresh.SessionRuns.Active.Length == 1, "runs can be read without context discovery");
        await fresh.RefreshAsync("hosts");
        Check(fresh.Error != null && fresh.Tabs.Selected == "hosts", "environment keeps its own read failure");
        await fresh.RefreshAsync("runs");
        Check(fresh.Error == null && fresh.Tabs.Selected == "runs", "returning to Runs clears the unrelated environment error");
        transport.Handler = (_, _) => throw new IOException("activity unavailable");
        await fresh.RefreshAsync("runs");
        Check(fresh.RunRefreshError == "activity unavailable" && fresh.SessionRuns.Active.Length == 1 && !fresh.Loading, "failed refresh preserves the last run list");
        transport.Handler = null; await fresh.RefreshAsync("runs");
        Check(fresh.RunRefreshError == null && !fresh.Loading, "explicit retry recovers");
        var noSession = new WorkspacePanelModel(new NativePanelClient(transport), "project-a", null,
            new NativePanelTabs(available: ["files", "memos", "runs"]), activity: new NativeContextActivityClient(transport));
        calls = transport.Calls.Count;
        await noSession.RefreshAsync("runs"); await noSession.OpenRunAsync("run-a");
        Check(!await noSession.PollRunsAsync(), "sessionless timer has no work");
        Check(noSession.Tabs.Selected == "runs" && noSession.SessionRuns.Empty && transport.Calls.Count == calls,
            "project without a conversation shows the Runs empty state without creating or reading a frame");
        await CheckPollingAsync();
        Console.WriteLine("Native Run navigation, polling, identity, cancellation, readonly and out-of-order checks passed.");
    }

    private static async Task CheckPollingAsync()
    {
        var homeHost = new Fake();
        var home = new WorkspacePanelModel(new NativePanelClient(homeHost), "project-a", "session-a",
            new NativePanelTabs(available: NativePanelTabs.All), activity: new NativeContextActivityClient(homeHost));
        Check(await home.PollRunsAsync() && home.Tabs.Selected == NativePanelTabs.Home && home.Tabs.Open.Count == 0,
            "background polling updates Start without opening a tool tab");
        home.Close();
        var host = new Fake(); var panel = Create(host);
        Check(await panel.PollRunsAsync() && panel.Tabs.Selected == "files", "background read updates badge without navigating");
        var reply = (JsonObject)host.Default("native_conversation_panel_activity", new())!;
        var run = (JsonObject)reply["runs"]![0]!; run["last_polled_at"] = 17;
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(reply);
        Check(!await panel.PollRunsAsync() && panel.Activity!.Runs[0].LastPolledAt == 17, "heartbeat advances without notifying card renderer");
        run["last_poll_error"] = "offline";
        Check(await panel.PollRunsAsync(), "poll error on a run is a visible change");
        host.Handler = (_, _) => throw new IOException("connection lost");
        Check(await panel.PollRunsAsync() && panel.RunRefreshError == "connection lost" && panel.Error == null && panel.SessionRuns.Active.Length == 1,
            "failed background read retains list without leaking errors into the Files tab");
        var pending = new TaskCompletionSource<JsonNode?>(); host.Handler = (_, _) => pending.Task;
        var calls = host.Calls.Count; var first = panel.PollRunsAsync();
        Check(!await panel.PollRunsAsync() && host.Calls.Count == calls + 1, "timer does not queue duplicate reads");
        var since = panel.RunReadStartedAt!.Value;
        Check(!panel.RunReadDelayed(since.AddSeconds(9)) && panel.RunReadDelayed(since.AddSeconds(10)), "only slow reads report a delay");
        var opening = panel.RefreshAsync("runs"); Check(host.Calls.Count == calls + 1, "manual navigation shares pending read");
        run["status"] = "succeeded"; pending.SetResult(reply); await Task.WhenAll(first, opening);
        Check(panel.RunRefreshError == null && panel.SessionRuns.Active.Length == 0 && panel.SessionRuns.Completed.Length == 1 && panel.RunReadStartedAt == null,
            "recovery removes warning and moves terminal row to Completed");
        host.Handler = (command, _) => command == "native_conversation_panel_run_cancel" ? throw new IOException("uncertain") : Task.FromResult<JsonNode?>(reply);
        await panel.CancelRunAsync("run-a"); await panel.PollRunsAsync();
        Check(panel.Error?.Contains("uncertain") == true && host.Calls.Count(c => c.Command == "native_conversation_panel_run_cancel") == 1,
            "poll never acknowledges or replays an uncertain mutation");
        var write = new TaskCompletionSource<JsonNode?>(); var stale = new TaskCompletionSource<JsonNode?>();
        host.Handler = (command, _) => command == "native_conversation_panel_run_cancel" ? write.Task : stale.Task;
        var changing = panel.CancelRunAsync("run-a");
        var reading = panel.RefreshAsync("runs");
        write.SetResult(Run("run-a")); await changing;
        stale.SetResult(host.Default("native_conversation_panel_activity", new())); await reading;
        Check(panel.SessionRuns.Completed.Length == 1 && panel.SessionRuns.Active.Length == 0,
            "explicit read started during a mutation cannot overwrite the list after the write settles");
        pending = new(); host.Handler = (_, _) => pending.Task;
        first = panel.PollRunsAsync(); panel.Close(); pending.SetException(new IOException("late"));
        Check(!await first && panel.RunRefreshError == null && !panel.RunPolling, "disposed pane ignores late poll failures");
    }

    private static async Task EnvironmentReadsAsync()
    {
        var host = new Fake(); var client = new NativePanelClient(host);
        var draft = new WorkspacePanelModel(client, "project-a", null, new NativePanelTabs(available: ["hosts"]));
        JsonObject Response(JsonNode data, string? session = null) => new() {
            ["project_id"] = "project-a", ["session_id"] = session, ["context_id"] = "local", ["data"] = data };
        var earlier = Run("earlier"); earlier["frame_id"] = "sibling";
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(Response(new JsonArray(earlier.DeepClone())));
        var rows = await draft.ReadContextRunsAsync("local", default);
        var read = host.Calls.Single();
        Check(rows.Single().FrameId == "sibling" && read.Command == "native_context_runs_list"
            && read.Project == "project-a" && read.Args["session_id"] == null && read.Args["context_id"]!.ToString() == "local"
            && !read.Args.ContainsKey("run_id"), "browsing from a draft lists the selected environment across frames with no session create or detail read");
        host.Handler = (_, args) => Task.FromResult<JsonNode?>(Response(earlier.DeepClone(), args["session_id"]?.ToString()));
        Check((await Create(host).ReadContextRunDetailAsync("local", "earlier", default)).Id == "earlier"
            && host.Calls.Last().Args["session_id"]!.ToString() == "session-a", "expanding output retains the requesting frame, not the record owner's frame");
        async Task Reject(Task operation)
        {
            try { await operation; throw new InvalidOperationException("Accepted mismatched environment run data"); }
            catch (InvalidDataException) { }
        }
        await Reject(draft.ReadContextRunDetailAsync("local", "unrequested", default));
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(Response(new JsonArray(), "foreign"));
        await Reject(draft.ReadContextRunsAsync("local", default));
        var active = rows.Single() with { CanManage = true };
        var frame = Create(host);
        JsonObject Artifact(string reader) => new() { ["run_id"] = "earlier", ["artifact_id"] = "a1", ["label"] = "QC summary",
            ["filename"] = "summary.tsv", ["kind"] = "text/tab-separated-values", ["reader_session"] = reader };
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(Response(new JsonArray(Artifact("sibling"))));
        var link = (await draft.ReadContextRunArtifactsAsync("local", default)).Single();
        var preview = draft.ContextArtifactPreview(link);
        Check(preview.Scope == "project-a:sibling" && preview.Find("a1")?.Path == "artifact:a1" && host.Calls.Last().Command == "native_context_run_artifacts",
            "a project without a selected session opens the registered artifact using the verified reader, not its graph title");
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(Response(new JsonArray(Artifact("sibling")), "session-a"));
        await Reject(frame.ReadContextRunArtifactsAsync("local", default));
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(Response(new JsonArray(Artifact("session-a"), Artifact("session-a")), "session-a"));
        await Reject(frame.ReadContextRunArtifactsAsync("local", default));
        Check(WorkspacePanelModel.CancelAction(active) == ContextRunAction.Cancel
            && WorkspacePanelModel.CancelAction(active with { Status = "cancelling" }) == ContextRunAction.ForceCancel
            && WorkspacePanelModel.CancelAction(active with { Kind = "method_search" }) == null, "control eligibility distinguishes ordinary, force and method-search cancellation");
        var terminal = active with { Status = "lost", RemoteWorkdir = "/fixture" };
        Check(WorkspacePanelModel.HasWorkspaceActions(terminal) && !WorkspacePanelModel.HasWorkspaceActions(terminal with { CleanedAt = 1 })
            && !WorkspacePanelModel.HasWorkspaceActions(terminal with { Kind = "file_transfer" }), "terminal workspace controls exclude cleaned and transferred data");
        var calls = host.Calls.Count;
        Check(!await frame.ChangeContextRunAsync("local", active with { CanManage = false }, ContextRunAction.Cancel, default)
            && !await frame.ChangeContextRunAsync("other", active, ContextRunAction.Cancel, default)
            && !await frame.ChangeContextRunAsync("local", active, ContextRunAction.ForceCancel, default) && host.Calls.Count == calls, "read-only, foreign context and stale action cannot send requests");
        earlier["context_id"] = "local"; earlier["can_manage"] = true;
        var beforeWrite = new TaskCompletionSource<JsonNode?>();
        host.Handler = (_, _) => beforeWrite.Task; var stale = frame.ReadContextRunsAsync("local", default, true);
        host.Handler = (_, _) => throw new IOException("lost reply");
        Check(!await frame.ChangeContextRunAsync("local", active, ContextRunAction.Cancel, default), "unconfirmed cancellation is not replayed");
        var actionCall = host.Calls.Last();
        Check(actionCall.Command == "native_context_run_action" && actionCall.Args["session_id"]!.ToString() == "session-a"
            && actionCall.Args["run_id"]!.ToString() == "earlier" && actionCall.Args["operation"]!["force"]!.GetValue<bool>() == false,
            "uses the requesting scope and sibling run without escalating intent");
        beforeWrite.SetResult(Response(new JsonArray(earlier.DeepClone()), "session-a")); await stale;
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(Response(new JsonArray(earlier.DeepClone()), "session-a"));
        await frame.ReadContextRunsAsync("local", default);
        Check(frame.RunActionPending(active.Id) && !await frame.ChangeContextRunAsync("local", active, ContextRunAction.Cancel, default),
            "neither a delayed explicit read nor polling unlocks an uncertain action");
        await frame.ReadContextRunsAsync("local", default, true);
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(Response(JsonValue.Create("earlier")!, "session-a"));
        Check(!frame.RunActionPending(active.Id) && await frame.ChangeContextRunAsync("local", terminal, ContextRunAction.Cleanup, default)
            && host.Calls.Last().Args["operation"]!.AsObject().Count == 1, "deliberate refresh allows safe cleanup with no force field");
        var review = frame.ContextRunReview("local", terminal, new NativeLocalization(new Dictionary<string, Dictionary<string,string>>()));
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(Response(new JsonObject { ["run_id"] = "earlier", ["acknowledged"]=true,
            ["listing"]=new JsonObject { ["entries"]=new JsonArray(), ["truncated"]=false } }, "session-a"));
        await review.OpenAsync("earlier"); calls = host.Calls.Count;
        Check(review.Ready && review.BeginConfirmation("cleanup") && review.HandleEscape() && review.Visible && review.Confirmation == null
            && review.HandleEscape() && !review.Visible && host.Calls.Count == calls, "window Escape dismisses confirmation and review separately without writing from an environment");
        earlier["context_id"] = "ssh:foreign";
        host.Handler = (_, _) => Task.FromResult<JsonNode?>(Response(new JsonArray(earlier.DeepClone())));
        await Reject(draft.ReadContextRunsAsync("local", default));
    }

    private static WorkspacePanelModel Create(Fake transport) => new(new NativePanelClient(transport), "project-a", "session-a",
        new NativePanelTabs("[\"files\"]", "files", NativePanelTabs.All), activity: new NativeContextActivityClient(transport));
    private static JsonObject Run(string id) => new()
    {
        ["id"] = id, ["frame_id"] = "session-a", ["context_id"] = "local", ["title"] = id,
        ["kind"] = "shell", ["status"] = "running", ["created_at"] = 1, ["progress_json"] = "{}", ["stdout_tail"] = "output of " + id
    };
    private static void Check(bool value, string message)
    { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public bool ReadOnly;
        public Func<string, JsonObject, Task<JsonNode?>>? Handler;
        public List<(string Command, string? Project, JsonObject Args)> Calls { get; } = [];
        public JsonNode? Default(string command, JsonObject args) => command switch
        {
            "native_conversation_panel_contexts" => new JsonObject { ["contexts"] = new JsonArray(), ["enabled_ids"] = new JsonArray(), ["read_only"] = ReadOnly },
            "list_ssh_hosts" => new JsonArray(),
            "native_conversation_panel_activity" => new JsonObject { ["runtimes"] = new JsonArray(), ["runs"] = new JsonArray(Run("run-a")), ["read_only"] = ReadOnly },
            "native_conversation_panel_files" => new JsonArray(),
            "native_conversation_panel_run_detail" or "native_conversation_panel_run_cancel" or "native_conversation_panel_run_harvest" => Run(args["run_id"]!.GetValue<string>()),
            _ => throw new InvalidOperationException(command)
        };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, projectId, (JsonObject)arguments.DeepClone()));
            return Handler?.Invoke(command, arguments) ?? Task.FromResult(Default(command, arguments));
        }
    }
}
