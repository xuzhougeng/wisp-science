using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeLiveAppearanceTests
{
    public static async Task RunAsync()
    {
        var host = new Fake(); var model = new NativeLiveAppearanceModel(host, "project-a");
        await model.LoadAsync(); var draft = model.Draft;
        Check(host.Writes.Count == 0, "loading does not persist defaults or rewrite preferences");
        model.Set("theme", JsonValue.Create("system"));
        Check(host.Writes.Count == 0, "unchanged appearance is a no-op");
        model.Set("theme", JsonValue.Create("dark"));
        Check(model.Draft!["theme"]!.GetValue<string>() == "dark" && model.Saving && host.Writes.Count == 1, "appearance changes synchronously while its first write is pending");
        model.Set("ui_font_size", JsonValue.Create(18)); model.Set("ui_font_size", JsonValue.Create(22)); model.Set("theme", JsonValue.Create("light"));
        Check(host.Writes.Count == 1, "edits behind a pending write are coalesced, not sent out of order");
        host.Complete(0, stored => { stored["send_with_modifier"] = true; stored["future_field"] = new JsonObject { ["value"] = 42 }; }); await host.WaitForWrites(2);
        Check(ReferenceEquals(draft, model.Draft) && model.Draft["theme"]!.GetValue<string>() == "light" && model.Draft["ui_font_size"]!.GetValue<int>() == 22, "older acknowledgement retains newer edits and control-bound object identity");
        Check(host.Writes[1]["theme"]!.GetValue<string>() == "light" && host.Writes[1]["ui_font_size"]!.GetValue<int>() == 22
            && host.Writes[1]["send_with_modifier"]!.GetValue<bool>() && host.Writes[1]["future_field"]!["value"]!.GetValue<int>() == 42,
            "queued edits patch a fresh host snapshot, preserving General and unknown fields");
        var leaving = model.WaitForPersistenceAsync();
        Check(!leaving.IsCompleted, "leaving does not cancel an accepted pending edit or permit an early stale readback");
        host.Complete(1); Check(await leaving && !model.Saving && !model.HasChanges, "settlement waits for the final write");
        Check(host.Projects.All(p => p == "project-a"), "all reads and writes retain explicit project scope");
        model.Set("code_font_family", JsonValue.Create("Cascadia Mono")); host.Fail(2);
        Check(!await model.WaitForPersistenceAsync() && model.Error != null && model.HasChanges, "failed response retains the applied values and exposes an error");
        var failedDraft = model.Draft;
        await model.LoadAsync();
        Check(ReferenceEquals(failedDraft, model.Draft) && model.Draft!["code_font_family"]!.GetValue<string>() == "Cascadia Mono"
            && model.Error != null && host.Writes.Count == 3, "reopening a retained appearance model keeps its failed draft and retry state without replay");
        model.Set("code_font_size", JsonValue.Create(16));
        Check(host.Writes.Count == 3 && model.Draft["code_font_size"]!.GetValue<int>() == 16, "further local edits do not automatically replay an uncertain write");
        model.Retry(); Check(host.Writes.Count == 4 && model.Error == null, "explicit retry persists the latest retained values");
        host.Complete(3); Check(await model.WaitForPersistenceAsync(), "explicit recovery clears the failed state");
        try { model.Set("workspace_dir", JsonValue.Create("unrelated")); throw new Exception("Expected unknown field rejection"); } catch (ArgumentException) { }
        Check(host.Writes.Count == 4, "live appearance cannot write unrelated preference fields");
        model.Set("custom_css", JsonValue.Create("@import url(remote);\n:root { --clay: red; }"));
        host.CompleteCss(4, "\n:root { --clay: red; }");
        Check(await model.WaitForPersistenceAsync() && model.Draft!["custom_css"]!.GetValue<string>() == "\n:root { --clay: red; }"
            && host.Writes.Count == 5, "canonical CSS from the shared host replaces the accepted draft without another write");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        try { await model.LoadAsync(cancelled.Token); throw new Exception("Expected cancellation"); } catch (OperationCanceledException) { }
        Check(!model.Saving, "cancelled initial reads do not occupy the persistence queue");
        await ImmediateInteractionAsync();
    }
    private static async Task ImmediateInteractionAsync()
    {
        var host = new Fake(); var live = new NativeLiveAppearanceModel(host, "interaction-project");
        await live.LoadAsync();
        using var general = new NativeSettingsEditorModel(host, "interaction-project");
        await general.LoadAsync("get_settings");
        general.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        general.Draft!["get_settings"]!["workspace_dir"] = "unsaved-folder";
        live.Set("send_with_modifier", JsonValue.Create(true));
        Check(NativeInputPreferences.From(live.Draft!).SendWithModifier && host.Writes.Count == 1,
            "shortcut applies immediately and persists without General Save");
        live.Set("selection_popup_enabled", JsonValue.Create(false));
        live.Set("theme", JsonValue.Create("dark"));
        Check(host.Writes.Count == 1 && !NativeInputPreferences.From(live.Draft!).SelectionPopupEnabled,
            "selection preference applies immediately while appearance and interaction share one queue");
        Check(general.CancelPreferences() && !general.HasChanges && live.Saving,
            "General Cancel discards its workspace draft without cancelling an accepted interaction write");
        host.Complete(0, stored => stored["future_field"] = "preserved"); await host.WaitForWrites(2);
        Check(!host.Writes[1]["selection_popup_enabled"]!.GetValue<bool>() && host.Writes[1]["send_with_modifier"]!.GetValue<bool>()
            && host.Writes[1]["theme"]!.GetValue<string>() == "dark" && host.Writes[1]["future_field"]!.GetValue<string>() == "preserved",
            "older acknowledgement cannot undo newer interaction edits or overwrite fresh unrelated preferences");
        host.Complete(1); Check(await live.WaitForPersistenceAsync(), "accepted input changes finish after leaving the General editor");
        general.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        general.Draft!["get_settings"]!["locale"] = "zh";
        Check(await general.SaveAsync() && host.SettingsWrites == 1 && host.Writes.Count == 2,
            "General Save writes ordinary settings without replaying or overwriting the immediate appearance fields");
        live.Set("selection_popup_enabled", JsonValue.Create(true)); host.Fail(2);
        Check(!await live.WaitForPersistenceAsync(), "uncertain interaction write is exposed");
        await live.LoadAsync(); live.Set("send_with_modifier", JsonValue.Create(false));
        Check(host.Writes.Count == 3 && live.Error != null && NativeInputPreferences.From(live.Draft!).SelectionPopupEnabled,
            "reopen and later edits retain failed interaction values without automatic replay");
        live.Retry(); host.Complete(3); Check(await live.WaitForPersistenceAsync(), "explicit interaction retry settles");
        Check(!host.Stored["send_with_modifier"]!.GetValue<bool>() && host.Stored["selection_popup_enabled"]!.GetValue<bool>()
            && host.Stored["theme"]!.GetValue<string>() == "dark" && host.Projects.All(p => p == "interaction-project"),
            "retry saves the latest interaction values while retaining appearance and project scope");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public JsonObject Stored = new() { ["theme"] = "system", ["ui_font_size"] = 14, ["send_with_modifier"] = false };
        public int SettingsWrites;
        private JsonObject settings = new() { ["locale"] = "en", ["workspace_dir"] = "original-folder" };
        public readonly List<JsonObject> Writes = [];
        public readonly List<string?> Projects = [];
        private readonly List<TaskCompletionSource<JsonNode?>> replies = [];
        private TaskCompletionSource countChanged = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            cancellationToken.ThrowIfCancellationRequested(); Projects.Add(projectId);
            if (command == "get_settings") return Task.FromResult<JsonNode?>(settings.DeepClone());
            if (command == "set_settings") { SettingsWrites++; settings = (JsonObject)args["settings"]!.DeepClone(); return Task.FromResult<JsonNode?>(null); }
            if (command == "get_appearance_prefs") return Task.FromResult<JsonNode?>(Stored.DeepClone());
            if (command != "set_appearance_prefs") throw new InvalidOperationException(command);
            Writes.Add((JsonObject)args["prefs"]!.DeepClone()); var reply = new TaskCompletionSource<JsonNode?>(TaskCreationOptions.RunContinuationsAsynchronously); replies.Add(reply);
            var changed = countChanged; countChanged = new(TaskCreationOptions.RunContinuationsAsynchronously); changed.SetResult();
            return reply.Task;
        }
        public void Complete(int index, Action<JsonObject>? afterWrite = null)
        { Stored = (JsonObject)Writes[index].DeepClone(); afterWrite?.Invoke(Stored); replies[index].SetResult(Writes[index].DeepClone()); }
        public void Fail(int index) => replies[index].SetException(new IOException("lost acknowledgement"));
        public void CompleteCss(int index, string css)
        { Stored = (JsonObject)Writes[index].DeepClone(); Stored["custom_css"] = css; replies[index].SetResult(Stored.DeepClone()); }
        public async Task WaitForWrites(int count)
        { while (Writes.Count < count) await countChanged.Task.WaitAsync(TimeSpan.FromSeconds(2)); }
    }
}
