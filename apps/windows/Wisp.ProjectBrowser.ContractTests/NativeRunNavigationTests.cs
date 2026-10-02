using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeRunNavigationTests
{
    public static async Task RunAsync()
    {
        var transport = new Fake();
        var model = Create(transport);
        await model.OpenRunAsync("run-a");
        Check(model.Tabs.Selected == "hosts" && model.SelectedRunId == "run-a" && model.RunDetail?.Id == "run-a" && !model.RunLoading,
            "message navigation opens the hosts tab and the exact run");
        Check(transport.Calls.All(call => call.Project == "project-a" && call.Args["session_id"]!.GetValue<string>() == "session-a"),
            "every read explicitly scopes project and session");
        Check(transport.Calls.All(call => call.Command is "native_conversation_panel_contexts" or "native_conversation_panel_activity" or "native_conversation_panel_run_detail"),
            "opening run details never starts a runtime or mutates a run");

        var slowTransport = new Fake();
        var slowModel = Create(slowTransport);
        var contexts = new TaskCompletionSource<JsonNode?>();
        var contextReads = 0;
        slowTransport.Handler = (command, args) => command == "native_conversation_panel_contexts" && ++contextReads == 1
            ? contexts.Task : Task.FromResult(slowTransport.Default(command, args));
        var oldOpen = slowModel.OpenRunAsync("first");
        await slowModel.OpenRunAsync("second");
        contexts.SetResult(slowTransport.Default("native_conversation_panel_contexts", new())); await oldOpen;
        Check(slowModel.RunDetail?.Id == "second" && slowTransport.Calls.Count(call => call.Command == "native_conversation_panel_run_detail") == 1,
            "superseded open stops before its detail read even when context loading was delayed");

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
        Check(model.Tabs.Selected == "files" && model.RunDetail == null && model.RunError == null && !model.RunLoading, "late errors cannot reopen details after leaving hosts");

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

        transport.Handler = null; await model.RefreshAsync("hosts");
        await model.CancelRunAsync("run-a");
        Check(!model.ActivityBusy && transport.Calls.Count(call => call.Command == "native_conversation_panel_run_cancel") == 1,
            "successful mutation releases busy after its own generation-changing refresh");
        var write = new TaskCompletionSource<JsonNode?>();
        transport.Handler = (command, args) => command == "native_conversation_panel_run_cancel" ? write.Task : Task.FromResult(transport.Default(command, args));
        var cancel = model.CancelRunAsync("run-a");
        await model.RefreshAsync("files");
        write.SetResult(Run("run-a")); await cancel;
        Check(!model.ActivityBusy && model.Tabs.Selected == "files", "late mutation does not reopen hosts or leave busy stuck");
        transport.Handler = null; await model.RefreshAsync("hosts");
        transport.Handler = (command, args) => Task.FromResult(command == "native_conversation_panel_run_cancel" ? Run("wrong-id") : transport.Default(command, args));
        var mutations = transport.Calls.Count(call => call.Command == "native_conversation_panel_run_cancel");
        await model.CancelRunAsync("run-a");
        Check(!model.ActivityBusy && model.Error?.Contains("身份") == true
            && transport.Calls.Count(call => call.Command == "native_conversation_panel_run_cancel") == mutations + 1,
            "mismatched mutation result is uncertain and never automatically replayed");
        transport.Handler = (command, args) => command == "native_conversation_panel_run_cancel" ? throw new OperationCanceledException() : Task.FromResult(transport.Default(command, args));
        await model.CancelRunAsync("run-a");
        Check(!model.ActivityBusy && model.Error?.Contains("未确认") == true, "cancelled mutation releases busy and reports uncertainty");
        transport.Handler = null; transport.ReadOnly = true; await model.RefreshAsync("hosts");
        calls = transport.Calls.Count; await model.CancelRunAsync("run-a"); await model.HarvestRunAsync("run-a");
        Check(transport.Calls.Count == calls, "read-only run panels reject mutations in the model");
        transport.ReadOnly = false; await model.RefreshAsync("hosts");
        delayed = new(); transport.Handler = (_, _) => delayed.Task;
        read = model.ReadRunAsync("run-a"); model.Close(); delayed.SetResult(Run("run-a")); await read;
        Check(model.RunDetail == null && model.SelectedRunId == null && !model.RunLoading, "disposed panel rejects late details");
        Console.WriteLine("Native Run navigation, identity, cancellation, readonly and out-of-order checks passed.");
    }

    private static WorkspacePanelModel Create(Fake transport) => new(new NativePanelClient(transport), "project-a", "session-a",
        new NativePanelTabs(selected: "files"), activity: new NativeContextActivityClient(transport));
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
