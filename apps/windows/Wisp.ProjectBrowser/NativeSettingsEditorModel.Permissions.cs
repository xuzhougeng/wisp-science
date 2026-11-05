using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed partial class NativeSettingsEditorModel
{
    protected override string UnconfirmedFailure(Exception exception) => Localization == null ? base.UnconfirmedFailure(exception)
        : Localization.Choose("操作结果未能确认；不会自动重试。", "The operation could not be confirmed; it will not be retried automatically.") + "\n" + exception.Message;
    public string ApprovalScope => Values["list_connectors"]?["scope"]?.GetValue<string>() ?? "ask";
    public bool ApprovalScopeAvailable => Values["list_connectors"] is JsonObject;
    public JsonObject[] ApprovalGrants => (Values["list_approval_grants"] as JsonArray)?.OfType<JsonObject>().ToArray() ?? [];
    public Task<bool> SetApprovalScopeAsync(string scope)
    {
        if (Busy || Closed || !ApprovalScopeAvailable || scope is not ("ask" or "auto" or "full")) return Task.FromResult(false);
        if (scope == ApprovalScope) return Task.FromResult(true);
        return RunAsync(() => client.InvokeAsync("set_approval_scope", new() { ["scope"] = scope }, projectId),
            _ => Values["list_connectors"]!["scope"] = scope);
    }
    private static bool SameGrant(JsonObject left, JsonObject right) =>
        new[] { "scope", "kind", "target", "session_id", "project_id" }.All(key => JsonNode.DeepEquals(left[key], right[key]));
    public Task<bool> RevokeApprovalAsync(JsonObject requested)
    {
        if (Busy || Closed) return Task.FromResult(false);
        var grant = ApprovalGrants.FirstOrDefault(row => SameGrant(row, requested));
        if (grant == null) return Task.FromResult(false);
        var identity = (JsonObject)grant.DeepClone();
        var args = new JsonObject
        {
            ["scope"] = identity["scope"]?.DeepClone(), ["kind"] = identity["kind"]?.DeepClone(), ["target"] = identity["target"]?.DeepClone(),
            ["sessionId"] = identity["session_id"]?.DeepClone(), ["projectId"] = identity["project_id"]?.DeepClone()
        };
        return RunAsync(() => client.InvokeAsync("revoke_approval_grant", args, projectId), _ =>
        {
            if (Values["list_approval_grants"] is JsonArray rows)
                foreach (var row in rows.OfType<JsonObject>().Where(row => SameGrant(row, identity)).ToArray()) rows.Remove(row);
        });
    }
    public Task<bool> RevokeAllApprovalsAsync()
    {
        if (Busy || Closed || ApprovalGrants.Length == 0) return Task.FromResult(false);
        return RunAsync(() => client.InvokeAsync("revoke_all_approval_grants", new(), projectId),
            _ => Values["list_approval_grants"] = new JsonArray());
    }
}
