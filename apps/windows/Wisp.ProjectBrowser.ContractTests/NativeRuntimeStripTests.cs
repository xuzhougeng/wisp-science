using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeRuntimeStripTests
{
    public static async Task RunAsync(string fixture)
    {
        var shared = JsonSerializer.Deserialize<NativeRuntimeSnapshot>(File.ReadAllText(fixture), ConversationSnapshot.JsonOptions)!;
        Check(shared.SessionId == null && shared.Runtimes[0].Key.ProjectId == shared.ProjectId
            && shared.Runtimes[0].Generation == 3 && shared.Contexts[0].Kind == "local", "shared Rust fixture preserves mixed wire casing");
        var local = new NativePanelContext("local", "local", "Local", "{}", "{}", null, null);
        var remote = new NativePanelContext("ssh:a", "ssh", "Cluster", "{\"python_path\":\"python\"}", "{}", "ok", null);
        var snapshot = new NativeRuntimeSnapshot("p", null, [local, remote], [], null, [], false);
        var slots = NativeRuntimeStripModel.BuildSlots(snapshot);
        Check(slots.Length == 2 && slots.All(s => s.Status == "missing"), "draft only shows local and does not presume detected R unavailable");
        slots = NativeRuntimeStripModel.BuildSlots(snapshot with { DefaultContextId = remote.Id });
        Check(slots.Length == 4 && slots.Last().Status == "unavailable", "remote global default appears without a persisted session");
        Check(!NativeRuntimeStripModel.AvailableLanguage(local with { LastProbeStatus = "ok" }, "r"), "a successful probe without R is unavailable");
        Check(!NativeRuntimeStripModel.AvailableLanguage(local with { CapabilitiesJson = "{\"rscript_executable\":\"Rscript\",\"r_jsonlite\":false}" }, "r"), "R requires jsonlite unless configured explicitly");
        NativeRuntimeInfo Info(string id, string session, ulong time) => new(id, 1, new("p", "local", "python", "mainline", session), "ready", null, null, null, 0, time, null, null);
        snapshot = snapshot with { SessionId = "s", Runtimes = [Info("shared", "", 9), Info("owned", "s", 2), Info("other", "other", 10)] };
        Check(NativeRuntimeStripModel.BuildSlots(snapshot)[0].Info?.RuntimeId == "owned", "conversation worker wins over newer shared and unrelated workers");
        var fake = new Fake(snapshot with { SessionId = null, Runtimes = [] }); var model = new NativeRuntimeStripModel(fake);
        model.Bind("p", null); await model.RefreshAsync();
        Check(model.AllIdle && fake.Calls.SequenceEqual(["native_runtimes_read"]) && fake.Args?["session_id"] == null, "opening draft reads only and never creates a frame or interpreter");
        var read = fake.PendingRead = new(); var pending = model.RefreshAsync(); model.Bind("other", null);
        read.SetResult(JsonSerializer.SerializeToNode(fake.Value, ConversationSnapshot.JsonOptions)); await pending;
        Check(model.Slots.Length == 0 && model.Snapshot == null, "late read cannot populate the next project");
        fake.PendingRead = null; model.Bind("p", null); await model.RefreshAsync();
        var action = fake.PendingAction = new(); var starting = model.ActAsync("start", model.Slots[0]);
        Check(model.Busy && fake.Args?["context_id"]?.ToString() == "local", "explicit start is bound to visible context");
        model.Bind("other", null); action.SetResult(null); await starting;
        Check(model.Snapshot == null && !model.Busy && fake.Calls.Count(c => c == "native_runtimes_start") == 1, "late action cannot retarget or replay");
        fake.PendingAction = null; model.Bind("p", null); fake.Value = fake.Value with { ReadOnly = true }; await model.RefreshAsync();
        var count = fake.Calls.Count; await model.ActAsync("start", model.Slots[0]);
        Check(fake.Calls.Count == count, "read-only scope rejects mutation locally");
        fake.Value = fake.Value with { ReadOnly = false }; await model.RefreshAsync();
        fake.FailAction = true; await model.ActAsync("start", model.Slots[0]);
        Check(model.Error != null && !model.Busy, "ambiguous mutation failure is exposed without automatic retry");
        count = fake.Calls.Count(c => c == "native_runtimes_start");
        await model.RefreshAsync();
        Check(fake.Calls.Count(c => c == "native_runtimes_start") == count, "recovery only reads state");
        fake.FailAction = false;
        read = fake.PendingRead = new(); pending = model.RefreshAsync();
        action = fake.PendingAction = new(); starting = model.ActAsync("start", model.Slots[0]);
        read.SetResult(JsonSerializer.SerializeToNode(fake.Value with { Runtimes = [Info("stale", "", 0)] }, ConversationSnapshot.JsonOptions)); await pending;
        Check(model.Slots.All(s => s.Info == null), "pre-action read cannot repaint stale runtime state during mutation");
        fake.PendingRead = null; fake.Value = fake.Value with { Runtimes = [Info("current", "", 1)] };
        action.SetResult(null); await starting;
        Check(model.Slots[0].Info?.RuntimeId == "current", "acknowledged mutation refreshes current runtime state");
        fake.PendingRead = new(); pending = model.RefreshAsync();
        using var interpreter = model.InterpreterEditor("local")!;
        interpreter.SetPath(true, "acknowledged-path");
        Check(await interpreter.SaveAsync(), "interpreter update returns a context acknowledgement");
        using var reopened = model.InterpreterEditor("local")!;
        Check(reopened.Values.Python == "acknowledged-path", "reopening immediately sees acknowledged paths without waiting for polling");
        fake.PendingRead.SetResult(JsonSerializer.SerializeToNode(fake.Value, ConversationSnapshot.JsonOptions)); await pending;
        using var afterOldRead = model.InterpreterEditor("local")!;
        Check(afterOldRead.Values.Python == "acknowledged-path", "a pre-save poll cannot roll back acknowledged interpreter paths");
        model.Bind("other", null); fake.PendingRead = null;
        interpreter.SetPath(true, "old-project"); await interpreter.SaveAsync();
        Check(model.Snapshot == null, "an old interpreter editor cannot populate the next project snapshot");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private sealed class Fake(NativeRuntimeSnapshot value) : INativeSettingsClient
    {
        public NativeRuntimeSnapshot Value = value; public List<string> Calls = []; public JsonObject? Args; public bool FailAction;
        public TaskCompletionSource<JsonNode?>? PendingRead, PendingAction;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken token = default)
        {
            Calls.Add(command); Args = (JsonObject)args.DeepClone();
            if (command == "native_runtimes_read") return PendingRead?.Task ?? Task.FromResult(JsonSerializer.SerializeToNode(Value, ConversationSnapshot.JsonOptions));
            if (command == "native_runtimes_start") return FailAction ? Task.FromException<JsonNode?>(new IOException("Ambiguous response")) : PendingAction?.Task ?? Task.FromResult<JsonNode?>(null);
            if (command == "update_execution_context_interpreters") return Task.FromResult(JsonSerializer.SerializeToNode(
                Value.Contexts.Single(c => c.Id == args["contextId"]!.ToString()) with { ConfigJson = new JsonObject
                { ["python_executable"] = args["pythonExecutable"]!.ToString(), ["rscript_executable"] = args["rscriptExecutable"]!.ToString() }.ToJsonString() }, ConversationSnapshot.JsonOptions));
            throw new InvalidOperationException("Unexpected command " + command);
        }
    }
}
