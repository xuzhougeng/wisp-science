using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed partial class NativeSettingsEditorModel
{
    public async Task<bool> TestConnectionAsync(Action<JsonArray> apply)
    {
        if (Busy || Closed || Draft == null || command is not ("add_mcp_connection" or "update_mcp_connection")) return false;
        if (NativeProxyRoutes.ErrorKey(Draft) is { } routeError) { Fail(routeError); return false; }
        var payload = NativeConnections.SaveDraft(Draft);
        OAuthPending = payload["transport"]?["kind"]?.GetValue<string>() == "http" && payload["transport"]?["auth"]?.GetValue<string>() == "oauth";
        try { return await InvokeAsync(OAuthPending ? "test_oauth_mcp_connection" : "test_mcp_connection", new() { ["conn"] = payload }, value => apply(ConnectionTools(value))); }
        finally { OAuthPending = false; if (!Closed) Notify(); }
    }
    public Task<bool> DiscoverConnectionToolsAsync(JsonObject source, Action<JsonArray> apply)
    {
        if (Draft != null || !((Values["list_mcp_connections"]?["connections"] as JsonArray)?.Contains(source) ?? false)) return Task.FromResult(false);
        return InvokeAsync("test_mcp_connection", new() { ["conn"] = NativeConnections.SaveDraft(source) }, value => apply(ConnectionTools(value)));
    }
    private static JsonArray ConnectionTools(JsonNode? value) => value is JsonArray rows && rows.All(row => row is JsonObject)
        ? (JsonArray)rows.DeepClone() : throw new InvalidDataException("连接未返回有效工具列表。");
}
