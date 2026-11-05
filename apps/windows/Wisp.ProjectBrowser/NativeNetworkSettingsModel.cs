using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Saves one network scope while retaining unsaved edits in every other scope.
public sealed class NativeNetworkSettingsModel(INativeSettingsClient client, string? projectId) : WorkspaceActionModel
{
    public static string[] ProxyScopes { get; } = ["model", "subscription", "mcp", "command"];
    public static string[] MirrorKeys { get; } = ["conda_mirror_url", "pip_index_url", "ca_bundle_path"];
    private readonly HashSet<string> custom = [];
    public JsonObject Saved { get; private set; } = new();
    public JsonObject Draft { get; private set; } = new();
    public bool Loaded { get; private set; }
    public bool MirrorsOpen { get; private set; }
    public bool SaveConfirmed { get; private set; }
    public bool HasChanges => Loaded && (!JsonNode.DeepEquals(Saved, Draft)
        || custom.Any(scope => Value(Draft, scope + "_proxy_url").Length == 0));
    public static string Value(JsonObject value, string key) => value[key]?.GetValue<string>() ?? "";
    public void AcceptSnapshot(JsonNode? value)
    {
        if (Busy || Closed || HasChanges || value is not JsonObject snapshot) return;
        Saved = (JsonObject)snapshot.DeepClone(); Draft = (JsonObject)snapshot.DeepClone(); Loaded = true;
    }
    public string Mode(string scope, bool saved = false)
    {
        var value = Value(saved ? Saved : Draft, scope + "_proxy_url");
        return value == "none" ? "direct" : value.Length > 0 || !saved && custom.Contains(scope) ? "custom" : "system";
    }
    public void SetMode(string scope, string mode)
    {
        if (Busy || Closed || !Loaded || !ProxyScopes.Contains(scope)) return;
        if (mode is not ("system" or "direct" or "custom")) throw new ArgumentException("Unknown proxy mode");
        custom.Remove(scope); if (mode == "custom") custom.Add(scope);
        Draft[scope + "_proxy_url"] = mode == "direct" ? "none" : ""; SaveConfirmed = false; ClearFailure(); Notify();
    }
    public void SetAddress(string scope, string value)
    {
        if (Busy || Closed || !Loaded || !ProxyScopes.Contains(scope)) return;
        custom.Add(scope); Draft[scope + "_proxy_url"] = value; SaveConfirmed = false; ClearFailure(); Notify();
    }
    public void SetMirror(string key, string value)
    {
        if (Busy || Closed || !Loaded || !MirrorKeys.Contains(key)) return;
        Draft[key] = value; SaveConfirmed = false; ClearFailure(); Notify();
    }
    public void OpenMirrors()
    {
        if (Busy || Closed || !Loaded) return;
        MirrorsOpen = true; SaveConfirmed = false; ClearFailure(); Notify();
    }
    public void CloseMirrors(bool cancel = false)
    {
        if (Busy || Closed) return;
        if (cancel) foreach (var key in MirrorKeys)
        {
            if (Saved.ContainsKey(key)) Draft[key] = Saved[key]?.DeepClone(); else Draft.Remove(key);
        }
        MirrorsOpen = false; Notify();
    }
    public void Discard()
    {
        if (Busy || Closed) return;
        Draft = (JsonObject)Saved.DeepClone(); custom.Clear(); SaveConfirmed = false; ClearFailure(); Notify();
    }
    public async Task<bool> SaveAsync(string scope)
    {
        if (!Loaded || Busy || Closed) return false;
        var keys = scope == "mirrors" ? MirrorKeys : ProxyScopes.Contains(scope) ? [scope + "_proxy_url"]
            : throw new ArgumentException("Unknown network scope");
        if (scope != "mirrors" && Mode(scope) == "custom" && string.IsNullOrWhiteSpace(Value(Draft, keys[0])))
        { Fail("network.address_required"); return false; }
        var next = (JsonObject)Saved.DeepClone();
        foreach (var key in keys) next[key] = Draft[key]?.DeepClone();
        SaveConfirmed = false;
        return await RunAsync(async () =>
        {
            var result = await client.InvokeAsync("set_network_settings", new() { ["settings"] = next }, projectId);
            var required = ProxyScopes.Select(s => s + "_proxy_url").Concat(MirrorKeys);
            if (result is not JsonObject confirmed || required.Any(key => confirmed[key] is not JsonValue value || !value.TryGetValue<string>(out _)))
                throw new InvalidDataException("The host did not confirm the saved network settings.");
            return confirmed;
        }, confirmed =>
        {
            // Rebase untouched fields on the acknowledgement without swallowing
            // another row's pending edit (including an empty custom proxy).
            var previous = Saved;
            Saved = (JsonObject)confirmed.DeepClone();
            foreach (var key in Saved.Select(pair => pair.Key).Union(Draft.Select(pair => pair.Key)).ToArray())
                if (keys.Contains(key) || JsonNode.DeepEquals(previous[key], Draft[key]))
                {
                    if (Saved.ContainsKey(key)) Draft[key] = Saved[key]?.DeepClone();
                    else Draft.Remove(key);
                }
            if (scope != "mirrors") custom.Remove(scope);
            SaveConfirmed = true;
        });
    }
    public override void Dispose() { base.Dispose(); Saved.Clear(); Draft.Clear(); custom.Clear(); }
}
