using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProxyRoutesTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var first = new JsonObject { ["id"] = "first", ["provider"] = "openai", ["api_url"] = "https://one.example.test/v1", ["proxy_url"] = "none" };
        var second = new JsonObject { ["id"] = "second", ["provider"] = "anthropic", ["api_url"] = "https://two.example.test", ["proxy_url"] = "http://localhost:7890" };
        var draft = NativeModelDrafts.EditDraft(first);
        Check(NativeProxyRoutes.ModelValue(draft, [first, second]) == "none", "untouched editor displays its endpoint route");
        draft["api_url"] = "https://TWO.example.test/v1/chat/completions/";
        Check(NativeProxyRoutes.ModelValue(draft, [first, second]) == "http://localhost:7890", "moving a model displays the destination API route across protocol suffixes");
        Check(!NativeModelDrafts.SaveArguments(draft)["profile"]!.AsObject().ContainsKey("proxy_url"), "untouched route must be omitted to preserve destination API configuration");
        NativeProxyRoutes.Set(draft, "", true);
        Check(NativeProxyRoutes.Mode("", true) == "custom" && NativeProxyRoutes.ErrorKey(draft) != null, "empty custom choice stays custom and cannot be submitted");
        NativeProxyRoutes.Set(draft, "", false);
        var payload = NativeModelDrafts.SaveArguments(draft)["profile"]!.AsObject();
        Check(payload.ContainsKey("proxy_url") && payload["proxy_url"]!.GetValue<string>() == "" && !payload.ContainsKey("_proxy_custom") && !payload.ContainsKey("_proxy_edited"), "explicit inheritance clears route without leaking UI fields");
        NativeProxyRoutes.Set(draft, " socks5h://localhost:1080 ", true);
        var test = NativeModelDrafts.TestArguments(new() { ["proxy_url"] = "http://global.example.test:7890", ["api_url"] = "" }, draft, NativeProxyRoutes.ModelValue(draft, [first, second]));
        Check(test["proxyUrl"]!.GetValue<string>() == "socks5h://localhost:1080" && test["settings"]!["proxy_url"]!.GetValue<string>() == "http://global.example.test:7890", "connection check routes separately from global legacy settings");
        Check(first["proxy_url"]!.GetValue<string>() == "none", "unsaved edits never mutate the stored profile");
        draft["restore_chat_model"] = true;
        var restored = NativeModelDrafts.SaveArguments(draft);
        Check(restored["restoreChatModel"]!.GetValue<bool>() && !restored["profile"]!.AsObject().ContainsKey("restore_chat_model"), "restoring chat is a command argument alongside the route, not an ignored profile field");
        var provider = NativeProviderDrafts.Create(); NativeProxyRoutes.Set(provider, "none", false);
        foreach (var entry in NativeProviderDrafts.Entries(provider))
            Check(NativeModelDrafts.SaveArguments(NativeProviderDrafts.Profile(provider, entry))["profile"]!["proxy_url"]!.GetValue<string>() == "none", "provider batch carries explicit endpoint route to every model");
        var connection = NativeConnections.Create(); NativeProxyRoutes.Set(connection, " http://localhost:7890 ", true);
        var clean = NativeConnections.SaveDraft(connection);
        Check(clean["proxy_url"]!.GetValue<string>() == "http://localhost:7890" && !clean.ContainsKey("_proxy_custom"), "MCP serializes the route separately from transport secrets");

        var fake = new Fake();
        using (var editor = new NativeSettingsEditorModel(fake, "fixture-project"))
        {
            editor.Edit(first, "save_model", "profile"); NativeProxyRoutes.Set(editor.Draft!, "", true);
            Check(!await editor.SaveAsync() && !await editor.TestModelAsync(_ => { }) && fake.Calls.Count == 0, "incomplete model route blocks both save and endpoint probe");
            editor.Discard(); editor.Edit(connection, "add_mcp_connection", "conn"); NativeProxyRoutes.Set(editor.Draft!, "", true);
            Check(!await editor.SaveAsync() && !await editor.TestConnectionAsync(_ => { }) && fake.Calls.Count == 0, "incomplete MCP route blocks writes and transport probes");
        }
        using var state = new NativeSubscriptionRoutesModel(fake, "fixture-project");
        Check(await state.LoadAsync() && state.Drafts.Count == 3 && !state.HasChanges, "subscription route load distinguishes loaded inheritance from unknown state");
        state.Set("chatgpt", "", true); var count = fake.Calls.Count;
        Check(!await state.SaveAsync("chatgpt") && state.HasChanges && fake.Calls.Count == count, "empty custom subscription route does not call the host");
        state.Set("chatgpt", "none", false); state.Set("codex", "http://localhost:7890", true);
        fake.Handler = (_, _) => throw new IOException("uncertain fixture");
        Check(!await state.SaveAsync("chatgpt") && state.HasChanges && !state.SaveConfirmed, "uncertain write preserves both drafts without retry");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject { ["chatgpt"] = "none" });
        Check(await state.SaveAsync("chatgpt") && state.SaveConfirmed && !state.ProviderHasChanges("chatgpt") && state.ProviderHasChanges("codex"), "confirmed write rebases only that provider and retains other unsaved routes");
        Check(fake.Calls.Last().Args["provider"]!.GetValue<string>() == "chatgpt" && fake.Calls.Last().Args["proxyUrl"]!.GetValue<string>() == "none" && fake.Calls.Last().Project == "fixture-project", "writes retain subscription identity, route and project binding");
        count = fake.Calls.Count; Check(!await state.LoadAsync() && fake.Calls.Count == count, "refresh cannot overwrite dirty subscription drafts");
        state.Discard(); Check(!state.HasChanges && state.Drafts["codex"]["proxy_url"]!.GetValue<string>() == "", "discard restores acknowledged routes");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonArray());
        Check(!await state.LoadAsync() && state.Error != null, "malformed status is reported, never treated as confirmed inheritance");
        var delayed = new TaskCompletionSource<JsonNode?>(); fake.Handler = (_, _) => delayed.Task;
        using var closing = new NativeSubscriptionRoutesModel(fake, null); var load = closing.LoadAsync(); closing.Dispose(); delayed.SetResult(new JsonObject());
        Check(!await load && closing.Saved == null && closing.Drafts.Count == 0, "late reply cannot resurrect a closed settings page");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<string, JsonObject, Task<JsonNode?>> Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject());
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId)); return Handler(command, arguments); }
    }
}
