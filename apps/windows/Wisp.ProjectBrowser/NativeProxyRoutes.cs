using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public static class NativeProxyRoutes
{
    public static string Text(JsonObject value, string key) => value[key]?.GetValue<string>() ?? "";
    public static string Mode(string value, bool custom) => value.Trim() switch { "none" => "direct", "" when !custom => "inherit", _ => "custom" };
    public static bool Custom(JsonObject draft) => draft["_proxy_custom"]?.GetValue<bool>() == true;
    public static string? ErrorKey(JsonObject draft) => Custom(draft) && string.IsNullOrWhiteSpace(Text(draft, "proxy_url")) ? "network.route_address_required" : null;
    public static void Set(JsonObject draft, string value, bool custom)
    { draft["proxy_url"] = value; draft["_proxy_custom"] = custom; draft["_proxy_edited"] = true; }
    public static string ModelValue(JsonObject draft, IEnumerable<JsonObject> profiles)
    {
        if (draft["_proxy_edited"]?.GetValue<bool>() == true) return Text(draft, "proxy_url");
        var endpoint = NativeProviderDrafts.NormalizeEndpoint(Text(draft, "api_url"));
        var candidates = profiles.Where(p => !NativeSubscriptionSettingsModel.IsSubscription(p)
            && NativeProviderDrafts.NormalizeEndpoint(Text(p, "api_url")) == endpoint).ToArray();
        return Text(candidates.FirstOrDefault(p => Text(p, "id") == Text(draft, "id")) ?? candidates.FirstOrDefault() ?? new(), "proxy_url");
    }
    public static void Clean(JsonObject payload, bool model)
    {
        if (model && payload["_proxy_edited"]?.GetValue<bool>() != true) payload.Remove("proxy_url");
        else if (payload.ContainsKey("proxy_url")) payload["proxy_url"] = Text(payload, "proxy_url").Trim();
        payload.Remove("_proxy_edited"); payload.Remove("_proxy_custom");
    }
}

/// Each subscription saves independently; loading and acknowledgement never swallow another draft.
public sealed class NativeSubscriptionRoutesModel(INativeSettingsClient client, string? projectId) : WorkspaceActionModel
{
    public JsonObject? Saved { get; private set; }
    public Dictionary<string, JsonObject> Drafts { get; } = [];
    public string? StatusProvider { get; private set; }
    public bool SaveConfirmed { get; private set; }
    public bool ProviderHasChanges(string provider) => Saved != null && Drafts.TryGetValue(provider, out var draft)
        && (NativeProxyRoutes.Custom(draft) || NativeProxyRoutes.Text(draft, "proxy_url") != NativeProxyRoutes.Text(Saved, provider));
    public bool HasChanges => Drafts.Keys.Any(ProviderHasChanges);
    public Task<bool> LoadAsync() => HasChanges ? Task.FromResult(false) : RunAsync(
        () => client.InvokeAsync("get_subscription_proxies", new(), projectId), value =>
        { Saved = Snapshot(value); Discard(); });
    private static JsonObject Snapshot(JsonNode? value) => value is JsonObject routes && routes.All(p => p.Value is JsonValue v && v.TryGetValue<string>(out _))
        ? (JsonObject)routes.DeepClone() : throw new InvalidDataException("Missing subscription proxy status.");
    public void Discard()
    {
        Drafts.Clear();
        if (Saved != null) foreach (var provider in NativeSubscriptionSettingsModel.Providers)
            Drafts[provider.Id] = new() { ["proxy_url"] = NativeProxyRoutes.Text(Saved, provider.Id) };
        StatusProvider = null; SaveConfirmed = false; ClearFailure(); Notify();
    }
    public void Set(string provider, string value, bool custom)
    { if (Busy || Closed || !Drafts.TryGetValue(provider, out var draft)) return; NativeProxyRoutes.Set(draft, value, custom); StatusProvider = null; SaveConfirmed = false; ClearFailure(); Notify(); }
    public Task<bool> SaveAsync(string provider)
    {
        if (Busy || Closed || Saved == null || !Drafts.TryGetValue(provider, out var draft)) return Task.FromResult(false);
        StatusProvider = provider; SaveConfirmed = false;
        if (NativeProxyRoutes.ErrorKey(draft) is { } key) { Fail(key); return Task.FromResult(false); }
        var submitted = NativeProxyRoutes.Text(draft, "proxy_url");
        return RunAsync(() => client.InvokeAsync("set_subscription_proxy", new() { ["provider"] = provider, ["proxyUrl"] = submitted }, projectId), value =>
        {
            var confirmed = Snapshot(value); var previous = Saved;
            Saved = confirmed;
            foreach (var id in Drafts.Keys.ToArray())
                if (id == provider || !NativeProxyRoutes.Custom(Drafts[id]) && NativeProxyRoutes.Text(Drafts[id], "proxy_url") == NativeProxyRoutes.Text(previous, id))
                    Drafts[id] = new() { ["proxy_url"] = NativeProxyRoutes.Text(confirmed, id) };
            SaveConfirmed = true;
        });
    }
    public override void Dispose() { base.Dispose(); Drafts.Clear(); Saved = null; }
}
