using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeRuntimeInterpreterTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        const string probe = "{\"python_executable\":\" probe-python \",\"rscript_executable\":\"probe-R\"}";
        var local = new NativePanelContext("local", "local", "", "{}", probe, "ok", null);
        Check(NativeRuntimeInterpreterValues.From("{\"python_path\":\" legacy \",\"rscript_executable\":\" explicit-R \"}", probe)
            == new NativeRuntimeInterpreterValues("legacy", "explicit-R"), "legacy and explicit overrides precede probed interpreters and trim whitespace");
        Check(NativeRuntimeInterpreterValues.From("{\"python_executable\":\" \",\"python_path\":\"ignored\"}", probe).Python == "probe-python",
            "empty canonical config falls through to probe, matching the shared Rust form");
        Check(NativeRuntimeInterpreterValues.From("invalid", "[]") == new NativeRuntimeInterpreterValues("", ""), "malformed config and non-object capabilities are safe");
        var fake = new Fake(local); var editor = new NativeRuntimeInterpreterModel(fake, "project", local);
        Check(editor.Values.Python == "probe-python" && editor.ContextLabel == "local" && editor.CanBrowse && fake.Calls == 0, "opening uses probe paths and makes no writes");
        editor.SetPath(true, "C:\\synthetic\\edited.exe"); editor.Dispose();
        Check(!await editor.SaveAsync() && fake.Calls == 0, "cancel discards a draft without saving");
        using var fresh = new NativeRuntimeInterpreterModel(fake, "project", local);
        var picker = new TaskCompletionSource<string?>(); var browse = fresh.BrowseAsync(true, () => picker.Task);
        Check(fresh.Browsing && !fresh.CanSave && !await fresh.SaveAsync() && fake.Calls == 0, "pending browse prevents an overlapping save");
        picker.SetResult(null); await browse;
        Check(fresh.Values.Python == "probe-python", "picker cancellation retains current path");
        await fresh.BrowseAsync(false, () => Task.FromResult<string?>("C:\\synthetic\\Rscript.exe"));
        Check(fresh.Values.Rscript.EndsWith("Rscript.exe"), "picker targets the requested interpreter only");
        fresh.SetPath(true, ""); fresh.SetPath(false, ""); fake.Pending = new(); var saving = fresh.SaveAsync();
        Check(!fresh.CanSave && !await fresh.SaveAsync() && fake.Calls == 1 && fake.Project == "project"
            && fake.Args!.ToJsonString() == "{\"contextId\":\"local\",\"pythonExecutable\":\"\",\"rscriptExecutable\":\"\"}",
            "clearing both fields sends one scoped camel-case request and never creates a session");
        fake.Pending.SetResult(JsonSerializer.SerializeToNode(local, ConversationSnapshot.JsonOptions));
        Check(await saving && fresh.Saved, "only a matching acknowledgement accepts the save");
        fake.Pending = null; fake.Fail = true; fresh.SetPath(true, "retain");
        Check(!await fresh.SaveAsync() && fresh.Error != null && fresh.Values.Python == "retain" && fake.Calls == 2, "save failure retains text without replay");
        fake.Fail = false; fake.Context = local with { Id = "wrong" };
        Check(!await fresh.SaveAsync() && !fresh.Saved, "mismatched acknowledgement is rejected");
        var late = new NativeRuntimeInterpreterModel(fake, "project", local); picker = new(); browse = late.BrowseAsync(true, () => picker.Task);
        late.Dispose(); picker.SetResult("late"); await browse;
        Check(late.Values.Python == "probe-python", "late picker cannot populate a closed or reopened form");
        var remote = new NativeRuntimeInterpreterModel(fake, "project", local with { Kind = "ssh" }); var picked = false;
        await remote.BrowseAsync(true, () => { picked = true; return Task.FromResult<string?>("bad"); });
        Check(!picked && !remote.CanBrowse && remote.CanSave, "remote paths are manual and cannot invoke a local picker"); remote.Dispose();
        fake.Pending = new(); var closed = new NativeRuntimeInterpreterModel(fake, "project", local); saving = closed.SaveAsync(); closed.Dispose();
        fake.Pending.SetResult(JsonSerializer.SerializeToNode(local, ConversationSnapshot.JsonOptions));
        Check(!await saving && !closed.Saved, "late save acknowledgement does not revive a disposed form");
    }
    private sealed class Fake(NativePanelContext context) : INativeSettingsClient
    {
        public NativePanelContext Context = context; public int Calls; public bool Fail; public JsonObject? Args; public string? Project;
        public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(command == "update_execution_context_interpreters", "only the explicit interpreter update is permitted");
            Calls++; Args = (JsonObject)args.DeepClone(); Project = projectId;
            if (Fail) return Task.FromException<JsonNode?>(new IOException("Synthetic lost acknowledgement"));
            return Pending?.Task ?? Task.FromResult(JsonSerializer.SerializeToNode(Context, ConversationSnapshot.JsonOptions));
        }
    }
}
