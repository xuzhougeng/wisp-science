using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeDecisionSettingsTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static JsonObject Catalog() => new()
    {
        ["presets"] = new JsonArray(
            new JsonObject { ["id"] = "hosted", ["base_url"] = "https://decision.invalid", ["model"] = "exact-model", ["timeout_secs"] = 10, ["key_required"] = true },
            new JsonObject { ["id"] = "local", ["base_url"] = "http://127.0.0.1:8090", ["model"] = "", ["timeout_secs"] = 60, ["key_required"] = false }),
        ["think_modes"] = new JsonArray("", "auto", "never", "always")
    };
    private static JsonObject Settings(bool key = false) => new()
    {
        ["enabled"] = false, ["preset"] = "hosted", ["base_url"] = "https://decision.invalid", ["model"] = "exact-model",
        ["think_mode"] = "", ["timeout_secs"] = 10, ["stored_keys"] = key ? new JsonArray("hosted") : new JsonArray()
    };
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var state = new NativeDecisionSettingsModel(fake, "project-one", Catalog());
        Check(await state.LoadAsync() && !state.HasChanges && state.SavedStatusKey == "decision.status_off", "decision load mounts a clean confirmed snapshot");
        state.Set("enabled", JsonValue.Create(true));
        Check(state.SavedStatusKey == "decision.status_off" && !state.SavedUsable, "unsaved enable does not claim the endpoint is in use");
        var before = fake.Calls.Count;
        Check(!await state.SaveAsync() && !await state.TestAsync() && fake.Calls.Count == before && state.ValidationKey == "decision.key_required",
            "hosted key preflight blocks writes and network probes");
        state.SetKey("synthetic-key"); state.Set("model", JsonValue.Create("edited"));
        Check(state.SelectPreset("local") && state.Draft!["enabled"]!.GetValue<bool>() && state.ApiKey == ""
            && state.TimeoutText == "60" && state.Draft["model"]!.GetValue<string>() == "", "preset switch keeps enable, replaces defaults and clears typed key");
        state.SetTimeout("-1"); before = fake.Calls.Count;
        Check(!await state.SaveAsync() && fake.Calls.Count == before && state.HasChanges, "invalid timeout cannot silently save the previous number");
        state.SetTimeout("0");
        fake.Handler = (command, args) => Task.FromResult<JsonNode?>(command == "test_decision_model"
            ? new JsonObject { ["model"] = "server-choice", ["elapsed_ms"] = 25 } : args["settings"]?.DeepClone());
        Check(await state.TestAsync() && state.HasChanges && state.Probe?["model"]?.GetValue<string>() == "server-choice"
            && state.SavedStatusKey == "decision.status_off", "explicit probe retains edits and never changes saved status");
        state.SetKey("synthetic-local-key");
        Check(await state.SaveAsync() && !state.HasChanges && state.ApiKey == "" && state.SavedUsable, "confirmed save replaces the snapshot and clears transient key");
        var call = fake.Calls.Last();
        Check(call.Command == "save_decision_model" && call.Project == "project-one" && call.Args["apiKey"]!.GetValue<string>() == "synthetic-local-key"
            && !call.Args["settings"]!.AsObject().ContainsKey("apiKey"), "existing host arguments keep keys separate from saved settings");
        state.Set("model", JsonValue.Create("keep-on-failure")); state.SetKey("draft-secret");
        fake.Handler = (_, _) => throw new IOException("uncertain write");
        Check(!await state.SaveAsync() && state.HasChanges && state.ApiKey == "draft-secret", "failed save keeps the draft and never retries automatically");
        state.Discard(); Check(!state.HasChanges && state.ApiKey == "", "leaving discards secrets and restores confirmed settings");
        await ClearAndCloseAsync();
        Console.WriteLine("Decision model draft, key boundary, probe and late-response checks passed.");
    }
    private static async Task ClearAndCloseAsync()
    {
        var fake = new Fake { Handler = (_, _) => Task.FromResult<JsonNode?>(Settings(key: true)) };
        using var state = new NativeDecisionSettingsModel(fake, null, Catalog()); await state.LoadAsync();
        state.Set("model", JsonValue.Create("unsaved-model")); state.SetKey("unsaved-replacement");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(Settings());
        Check(await state.ClearKeyAsync() && !NativeDecisionSettingsModel.HasKey(state.Draft!)
            && state.Draft!["model"]!.GetValue<string>() == "unsaved-model" && state.ApiKey == "unsaved-replacement" && state.HasChanges,
            "clearing one stored key preserves typed replacements and unrelated edits");
        var clear = fake.Calls.Last(); Check(clear.Command == "clear_decision_model_key" && clear.Args.Count == 1
            && clear.Args["preset"]!.GetValue<string>() == "hosted", "key clear targets only the selected preset");
        var pending = new TaskCompletionSource<JsonNode?>(); fake.Handler = (_, _) => pending.Task;
        var save = state.SaveAsync(); var before = fake.Calls.Count;
        Check(!await state.TestAsync() && !await state.SaveAsync() && fake.Calls.Count == before, "pending writes block competing writes and probes");
        state.Dispose(); pending.SetResult(Settings(key: true));
        Check(!await save && state.Saved == null && state.Draft == null && state.ApiKey == "", "late save cannot repopulate a closed editor");
        using var loading = new NativeDecisionSettingsModel(fake, null, Catalog()); pending = new();
        var load = loading.LoadAsync(); loading.Dispose(); pending.SetResult(Settings());
        Check(!await load && loading.Draft == null, "late load does not mount a closed page");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<string, JsonObject, Task<JsonNode?>>? Handler;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            return Handler?.Invoke(command, arguments) ?? Task.FromResult<JsonNode?>(Settings());
        }
    }
}
