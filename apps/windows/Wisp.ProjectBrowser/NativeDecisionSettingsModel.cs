using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// One decision endpoint draft. Keys travel only in the existing command's
/// separate apiKey argument; status always describes the confirmed snapshot.
public sealed class NativeDecisionSettingsModel(INativeSettingsClient client, string? projectId, JsonObject catalog) : WorkspaceActionModel
{
    public JsonObject? Saved { get; private set; }
    public JsonObject? Draft { get; private set; }
    public string ApiKey { get; private set; } = "";
    public string TimeoutText { get; private set; } = "";
    public string? MessageKey { get; private set; }
    public string? ValidationKey { get; private set; }
    public JsonObject? Probe { get; private set; }
    public IEnumerable<JsonObject> Presets => (catalog["presets"] as JsonArray)?.OfType<JsonObject>() ?? [];
    public string[] ThinkModes => (catalog["think_modes"] as JsonArray ?? []).Select(value => value!.GetValue<string>()).ToArray();
    public static string Text(JsonObject value, string key) => value[key]?.GetValue<string>() ?? "";
    public JsonObject? Preset(string id) => Presets.FirstOrDefault(value => Text(value, "id") == id);
    public JsonObject? SelectedPreset => Draft == null ? null : Preset(Text(Draft, "preset"));
    public static bool HasKey(JsonObject settings) => (settings["stored_keys"] as JsonArray ?? []).Any(value => value?.GetValue<string>() == Text(settings, "preset"));
    public bool TimeoutValid => ulong.TryParse(TimeoutText, out var value) && value <= 600;
    public bool HasChanges => Draft != null && (!JsonNode.DeepEquals(Saved, Draft) || ApiKey.Length > 0 || !TimeoutValid);
    public bool SavedUsable => Saved is { } value && value["enabled"]?.GetValue<bool>() == true
        && Preset(Text(value, "preset")) is { } preset && (preset["key_required"]?.GetValue<bool>() != true || HasKey(value));
    public string SavedStatusKey => Saved?["enabled"]?.GetValue<bool>() != true ? "decision.status_off"
        : SavedUsable ? "decision.status_on" : "decision.status_no_key";
    private void ClearMessage() { ClearFailure(); MessageKey = null; ValidationKey = null; Probe = null; }
    private void Accept(JsonObject value)
    {
        Saved = (JsonObject)value.DeepClone(); Draft = (JsonObject)value.DeepClone();
        ApiKey = ""; TimeoutText = value["timeout_secs"]?.ToString() ?? "0";
    }
    private static JsonObject Snapshot(JsonNode? value) => value is JsonObject settings
        && settings["preset"] is JsonValue && settings["stored_keys"] is JsonArray
        ? settings : throw new InvalidDataException("Missing decision model settings.");
    public Task<bool> LoadAsync() => RunAsync(async () => Snapshot(await client.InvokeAsync("get_decision_model", new(), projectId)),
        value => { Accept(value); ClearMessage(); });
    public void Set(string key, JsonNode? value)
    {
        if (Busy || Closed || Draft == null) return;
        Draft[key] = value?.DeepClone(); ClearMessage(); Notify();
    }
    public void SetKey(string value)
    {
        if (Busy || Closed || Draft == null) return;
        ApiKey = value; ClearMessage(); Notify();
    }
    public void SetTimeout(string value)
    {
        if (Busy || Closed || Draft == null) return;
        TimeoutText = value;
        if (ulong.TryParse(value, out var parsed) && parsed <= 600) Draft["timeout_secs"] = parsed;
        ClearMessage(); Notify();
    }
    public bool SelectPreset(string id)
    {
        if (Busy || Closed || Draft == null || Preset(id) is not { } preset) return false;
        if (id == Text(Draft, "preset")) return false;
        Draft = new JsonObject { ["enabled"] = Draft["enabled"]?.DeepClone(), ["stored_keys"] = Draft["stored_keys"]?.DeepClone(),
            ["preset"] = id, ["base_url"] = preset["base_url"]?.DeepClone(), ["model"] = preset["model"]?.DeepClone(),
            ["think_mode"] = "", ["timeout_secs"] = preset["timeout_secs"]?.DeepClone() };
        ApiKey = ""; TimeoutText = Draft["timeout_secs"]?.ToString() ?? "0"; ClearMessage(); Notify(); return true;
    }
    private bool Validate(bool test)
    {
        if (Busy || Closed || Draft == null || Saved == null) return false;
        ClearMessage();
        if (!TimeoutValid) { ValidationKey = "timeout"; Notify(); return false; }
        if ((test || Draft["enabled"]?.GetValue<bool>() == true) && SelectedPreset?["key_required"]?.GetValue<bool>() == true
            && !HasKey(Draft) && string.IsNullOrWhiteSpace(ApiKey))
        { ValidationKey = "decision.key_required"; Notify(); return false; }
        return true;
    }
    private JsonObject Arguments() => new() { ["settings"] = Draft!.DeepClone(), ["apiKey"] = ApiKey };
    public Task<bool> SaveAsync()
    {
        if (!Validate(test: false)) return Task.FromResult(false);
        var args = Arguments();
        return RunAsync(async () => Snapshot(await client.InvokeAsync("save_decision_model", args, projectId)),
            value => { Accept(value); MessageKey = "decision.saved"; });
    }
    public Task<bool> TestAsync()
    {
        if (!Validate(test: true)) return Task.FromResult(false);
        var args = Arguments(); MessageKey = "decision.testing";
        return RunAsync(async () => await client.InvokeAsync("test_decision_model", args, projectId) as JsonObject
                ?? throw new InvalidDataException("Missing decision probe result."),
            value => { Probe = (JsonObject)value.DeepClone(); MessageKey = "decision.test_ok"; });
    }
    public Task<bool> ClearKeyAsync()
    {
        if (Busy || Closed || Draft == null || !HasKey(Draft)) return Task.FromResult(false);
        ClearMessage(); var args = new JsonObject { ["preset"] = Text(Draft, "preset") };
        return RunAsync(async () => Snapshot(await client.InvokeAsync("clear_decision_model_key", args, projectId)), value =>
        {
            // Clearing a preset key must not erase unrelated unsaved form edits.
            Saved = (JsonObject)value.DeepClone(); Draft!["stored_keys"] = value["stored_keys"]!.DeepClone();
            MessageKey = "decision.key_cleared";
        });
    }
    public void Discard()
    {
        if (Busy || Closed) return;
        if (Saved != null) Accept(Saved); else { Draft = null; ApiKey = ""; TimeoutText = ""; }
        ClearMessage(); Notify();
    }
    public override void Dispose() { base.Dispose(); ApiKey = ""; Draft = null; Saved = null; Probe = null; }
}
