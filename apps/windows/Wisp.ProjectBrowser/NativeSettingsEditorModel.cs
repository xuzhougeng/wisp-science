using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// A mounted, project-bound settings editor. Only confirmed writes discard its draft.
public sealed class NativeSettingsEditorModel(INativeSettingsClient client, string? projectId) : WorkspaceActionModel
{
    public JsonObject Values { get; private set; } = new();
    public JsonObject? Draft { get; private set; }
    private JsonObject? original;
    private string? command, parameter;
    private JsonObject extra = new();
    public bool HasChanges => Draft != null && !JsonNode.DeepEquals(original, Draft);
    public bool OAuthPending { get; private set; }
    private bool cancellingOAuth;
    public Task<bool> LoadAsync(params string[] commands) => RunAsync(async () =>
    {
        var loaded = new JsonObject();
        foreach (var read in commands)
            loaded[read] = (await client.InvokeAsync(read, new(), projectId))?.DeepClone();
        return loaded;
    }, loaded => Values = loaded);

    public void Edit(JsonObject draft, string write, string? argument = null, JsonObject? arguments = null)
    {
        if (Busy || Closed) return;
        original = (JsonObject)draft.DeepClone(); Draft = (JsonObject)draft.DeepClone();
        command = write; parameter = argument; extra = (JsonObject?)arguments?.DeepClone() ?? new();
    }
    public void Discard() { if (!Busy) { Draft = null; original = null; command = null; } }
    public async Task<bool> SaveAsync()
    {
        if (Busy || Closed || Draft == null || command == null) return false;
        var args = (JsonObject)extra.DeepClone();
        if (command == "install_plugin" && string.IsNullOrWhiteSpace(Draft["expected_sha256"]?.GetValue<string>())) Draft["expected_sha256"] = null;
        if (command == "save_model")
            foreach (var field in NativeModelDrafts.SaveArguments(Draft)) args[field.Key] = field.Value?.DeepClone();
        else if (parameter != null) args[parameter] = Draft.DeepClone();
        else foreach (var field in Draft) args[Camel(field.Key)] = field.Value?.DeepClone();
        var operation = command is "add_mcp_connection" or "update_mcp_connection" && Draft["transport"]?["auth"]?.GetValue<string>() == "oauth"
            ? "authorize_http_connection" : command;
        OAuthPending = operation == "authorize_http_connection";
        try
        {
            return await RunAsync(() => client.InvokeAsync(operation, args, projectId), _ =>
            { Draft = null; original = null; command = null; });
        }
        finally { OAuthPending = false; if (!Closed) Notify(); }
    }
    public async Task CancelOAuthAsync()
    {
        if (!OAuthPending || Closed || cancellingOAuth) return;
        cancellingOAuth = true;
        try { await client.InvokeAsync("cancel_oauth_authorization", new(), projectId); }
        catch (Exception ex) { if (!Closed) Fail("取消授权未确认；不会自动重试。\n" + ex.Message); }
        finally { cancellingOAuth = false; }
    }
    public Task<bool> InvokeAsync(string operation, JsonObject args, Action<JsonNode?>? apply = null)
        => RunAsync(() => client.InvokeAsync(operation, (JsonObject)args.DeepClone(), projectId), value => apply?.Invoke(value));
    public Task<bool> TestModelAsync(Action<JsonNode?> apply)
    {
        if (Draft == null || command != "save_model" || Busy || Closed) return Task.FromResult(false);
        var draft = (JsonObject)Draft.DeepClone();
        return RunAsync(async () =>
        {
            var settings = await client.InvokeAsync("get_settings", new(), projectId) as JsonObject
                ?? throw new InvalidDataException("无法读取当前设置。");
            if (Closed) return null;
            return await client.InvokeAsync("validate_settings", NativeModelDrafts.TestArguments(settings, draft), projectId);
        }, apply);
    }
    public override void Dispose() { base.Dispose(); Draft = null; original = null; Values.Clear(); }
    private static string Camel(string key)
    {
        var words = key.Split('_');
        return words[0] + string.Concat(words.Skip(1).Select(w => w.Length == 0 ? "" : char.ToUpperInvariant(w[0]) + w[1..]));
    }
}
