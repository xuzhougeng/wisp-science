using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProviderSettingsTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static string S(JsonObject value, string key) => NativeProviderDrafts.Text(value, key);
    public static async Task RunAsync()
    {
        var draft = NativeProviderDrafts.Create();
        Check(NativeProviderDrafts.Entries(draft).Select(row => S(row, "model")).SequenceEqual(["deepseek-v4-flash", "deepseek-v4-pro"]),
            "Add API access begins with the WebView DeepSeek pair");
        Check(NativeProviderDrafts.ChangeUrl(draft, "https://api.openai.com/v1") && NativeProviderDrafts.Entries(draft).All(row => S(row, "provider") == "openai_responses")
            && NativeProviderDrafts.Entries(draft).Last()["use_for_image_generation"]!.GetValue<bool>(), "pristine URL changes regenerate protocol and media suggestions");
        var first = NativeProviderDrafts.Entries(draft).First(); var stableRow = S(first, "row_id"); first["label"] = "Custom display name";
        Check(!NativeProviderDrafts.ChangeUrl(draft, "https://api.anthropic.com") && S(NativeProviderDrafts.Entries(draft).First(), "row_id") == stableRow
            && S(first, "label") == "Custom display name", "changing an endpoint never overwrites edited rows");
        draft = NativeProviderDrafts.Create(); first = NativeProviderDrafts.Entries(draft).First(); NativeProviderDrafts.SetRole(first, "use_for_video_generation", true);
        Check(!NativeProviderDrafts.Pristine(draft), "video assignment also makes a row non-pristine");
        foreach (var (url, expected) in new[] { ("https://api.anthropic.com", "claude-sonnet-5"), ("https://api.kimi.com/coding/v1", "kimi-coding"),
            ("https://api.moonshot.cn/v1", "kimi-k3"), ("https://open.bigmodel.cn/api/coding/paas/v4", "glm-5.2"), ("https://open.bigmodel.cn/api/paas/v4", "glm-5") })
            Check(S(NativeProviderDrafts.Suggested(url).OfType<JsonObject>().First(), "model") == expected, "preset suggestions match " + url);
        Check(NativeProviderDrafts.Suggested("https://api.openai.com.evil.invalid").OfType<JsonObject>().Single()["model"]!.GetValue<string>() == "",
            "lookalike hosts do not inherit another provider's suggestions");
        var chat = NativeProviderDrafts.Entry(model: "chat"); var expert = NativeProviderDrafts.Entry(model: "expert"); expert["use_for_expert"] = true;
        var image = NativeProviderDrafts.Entry(model: "vendor/gpt-image-2"); var second = NativeProviderDrafts.Entry(model: "second-chat");
        draft = NativeProviderDrafts.Create(); draft["entries"] = new JsonArray(chat, expert, image, second, NativeProviderDrafts.Entry());
        Check(NativeProviderDrafts.SaveOrder(draft).Select(row => S(row, "model")).SequenceEqual(["vendor/gpt-image-2", "expert", "second-chat", "chat"]),
            "save order creates media first and the first everyday model last while skipping blank rows");
        NativeProviderDrafts.SetRole(chat, "use_for_vision", true); Check(chat["supports_vision"]!.GetValue<bool>(), "image analysis enables vision input");
        NativeProviderDrafts.SetRole(chat, "use_for_video_generation", true);
        Check(!chat["supports_vision"]!.GetValue<bool>() && !chat["use_for_vision"]!.GetValue<bool>() && !chat["use_for_image_generation"]!.GetValue<bool>(),
            "video assignment clears conflicting provider-row roles");
        draft = NativeProviderDrafts.Create("https://EXAMPLE.invalid/base/v1/");
        var stored = NativeModelDrafts.Create("https://example.invalid/base/v1/chat/completions", "saved"); stored["has_api_key"] = true; stored["id"] = "stored";
        Check(NativeProviderDrafts.StoredKeyProfile(draft, [stored]) == stored, "stored credentials match normalized host and protocol suffix");
        Check(!NativeProviderDrafts.NeedsKey(draft, [stored]) && NativeProviderDrafts.NeedsKey(draft, []),
            "opening new API access requests key focus only when the endpoint has no reusable key");
        draft["key"] = "fixture";
        Check(!NativeProviderDrafts.NeedsKey(draft, []), "an entered key is not missing");
        draft["key"] = "";
        stored["provider"] = "openai_codex";
        Check(NativeProviderDrafts.StoredKeyProfile(draft, [stored]) == null, "subscription credentials are never offered for API reuse");
        stored["provider"] = "openai"; stored["api_url"] = "http://example.invalid/base";
        Check(NativeProviderDrafts.StoredKeyProfile(draft, [stored]) == null, "credential reuse keeps the URL scheme distinct");
        draft = NativeProviderDrafts.Create(); draft["key"] = "synthetic-key";
        Check(NativeProviderDrafts.ErrorKey(draft, []) == null, "valid shared access can be saved");
        draft["api_url"] = " "; Check(NativeProviderDrafts.ErrorKey(draft, []) == "err.api_url_required", "blank shared URL blocks save");
        draft["api_url"] = "https://example.invalid"; draft["entries"] = new JsonArray(NativeProviderDrafts.Entry());
        Check(NativeProviderDrafts.ErrorKey(draft, []) == "err.model_required", "at least one named model is required");
        draft["entries"] = new JsonArray(NativeProviderDrafts.Entry(model: "gpt-image-2"));
        Check(NativeProviderDrafts.ErrorKey(draft, []) == "models.need_chat", "media-only setup still requires an everyday model");
        draft = NativeProviderDrafts.Create(); Check(NativeProviderDrafts.ErrorKey(draft, []) == "err.api_key_required", "an unstored endpoint requires its key");
        await SaveAndRetryAsync(); await TestAndCloseAsync();
        Console.WriteLine("Provider form suggestions, credentials, roles, batch save and late response checks passed.");
    }
    private static async Task SaveAndRetryAsync()
    {
        var fake = new Fake(); using var editor = new NativeSettingsEditorModel(fake, "provider-project");
        var draft = NativeProviderDrafts.Create(); draft["key"] = "synthetic-key"; draft["user_agent"] = "synthetic-agent"; draft["send_session_id"] = null;
        fake.Handler = (command, _) => Task.FromResult<JsonNode?>(command == "model_catalog_lookup" ? new JsonObject { ["max_tokens"] = 4200, ["context_window"] = 9000 } : new JsonArray());
        editor.Edit(draft, "save_provider");
        Check(await editor.SaveAsync() && editor.Draft == null && editor.SaveConfirmed, "confirmed batch save closes its editor");
        var saves = fake.Calls.Where(call => call.Command == "save_model").ToArray();
        Check(saves.Length == 2 && saves.All(call => call.Project == "provider-project")
            && saves[0].Args["profile"]!["model"]!.GetValue<string>() == "deepseek-v4-pro" && saves[1].Args["profile"]!["model"]!.GetValue<string>() == "deepseek-v4-flash",
            "batch writes preserve project binding and leave the first everyday model active");
        Check(saves.All(call => call.Args["key"]!.GetValue<string>() == "synthetic-key" && !call.Args["profile"]!.AsObject().ContainsKey("key")
            && call.Args["profile"]!["max_tokens"]!.GetValue<int>() == 4200 && call.Args["profile"]!["user_agent"]!.GetValue<string>() == "synthetic-agent"
            && !call.Args["profile"]!.AsObject().ContainsKey("row_id") && call.Args["profile"]!["send_session_id"] == null),
            "every model receives shared headers and exact catalog limits; credentials stay outside profile data");
        Check(fake.Calls.Where(call => call.Command == "model_catalog_lookup").Select(call => call.Args["model"]!.GetValue<string>()).SequenceEqual(["deepseek-v4-pro", "deepseek-v4-flash"]),
            "capacity lookup uses each exact model ID");
        fake.Calls.Clear(); var writes = 0;
        fake.Handler = (command, _) =>
        {
            if (command == "model_catalog_lookup") throw new IOException("offline catalog");
            if (++writes == 2) throw new IOException("lost acknowledgement");
            return Task.FromResult<JsonNode?>(new JsonArray());
        };
        editor.Edit(draft, "save_provider"); Check(!await editor.SaveAsync() && editor.Draft != null, "partial batch failure retains the full draft");
        var failedId = fake.Calls.Last().Args["profile"]!["id"]!.GetValue<string>();
        var before = fake.Calls.Count;
        Check(await editor.SaveAsync(), "an explicit retry can finish the remaining batch");
        var retried = fake.Calls.Skip(before).Where(call => call.Command == "save_model").ToArray();
        Check(retried.Length == 1 && retried[0].Args["profile"]!["id"]!.GetValue<string>() == failedId
            && retried[0].Args["profile"]!["max_tokens"]!.GetValue<int>() == 8192,
            "confirmed rows are skipped and uncertain rows retain their stable ID with offline capacity defaults");
        fake.Calls.Clear(); draft = NativeProviderDrafts.Create(); editor.Edit(draft, "save_provider");
        Check(!await editor.SaveAsync() && fake.Calls.Count == 0, "missing credentials never submit a batch");
    }
    private static async Task TestAndCloseAsync()
    {
        var fake = new Fake(); using var editor = new NativeSettingsEditorModel(fake, "provider-project");
        var saved = NativeModelDrafts.Create("https://api.deepseek.com/v1", "stored-model"); saved["id"] = "stored-id"; saved["has_api_key"] = true;
        editor.Values["list_models"] = new JsonArray(saved);
        fake.Handler = (command, _) => Task.FromResult<JsonNode?>(command == "get_settings" ? new JsonObject
        { ["model"] = "other", ["provider"] = "other", ["api_url"] = "old", ["endpoint_suffix"] = "", ["max_tokens"] = 1, ["has_api_key"] = false, ["workspace_dir"] = "keep" } : JsonValue.Create("ok"));
        var draft = NativeProviderDrafts.Create(); var row = NativeProviderDrafts.Entries(draft).First(); row["endpoint_suffix"] = "anthropic";
        editor.Edit(draft, "save_provider"); Check(await editor.TestModelAsync(_ => { }) && editor.Draft != null, "explicit provider test retains its unsaved rows");
        var test = fake.Calls.Last();
        Check(test.Command == "validate_settings" && test.Args["profileId"]!.GetValue<string>() == "stored-id"
            && test.Args["settings"]!["model"]!.GetValue<string>() == "deepseek-v4-flash" && test.Args["settings"]!["endpoint_suffix"]!.GetValue<string>() == "anthropic"
            && test.Args["settings"]!["has_api_key"]!.GetValue<bool>() && test.Args["settings"]!["workspace_dir"]!.GetValue<string>() == "keep"
            && fake.Calls.All(call => call.Command != "save_model"), "test uses the first row and correct stored-key profile without writing settings");
        var pending = new TaskCompletionSource<JsonNode?>(); fake.Handler = (_, _) => pending.Task;
        draft["key"] = "synthetic-key"; editor.Edit(draft, "save_provider");
        var saving = editor.SaveAsync(); var count = fake.Calls.Count;
        Check(!await editor.SaveAsync() && fake.Calls.Count == count, "a pending batch rejects concurrent saves");
        editor.Dispose(); pending.SetResult(new JsonObject { ["max_tokens"] = 20, ["context_window"] = 5000 });
        Check(!await saving && fake.Calls.Count == count, "closing during catalog lookup prevents any subsequent profile write");
        using var afterWrite = new NativeSettingsEditorModel(fake, null); pending = new();
        fake.Handler = (command, _) => command == "save_model" ? pending.Task : Task.FromResult<JsonNode?>(null);
        afterWrite.Edit(draft, "save_provider"); saving = afterWrite.SaveAsync(); count = fake.Calls.Count; afterWrite.Dispose(); pending.SetResult(new JsonArray());
        Check(!await saving && fake.Calls.Count == count && afterWrite.Draft == null, "closing during a batch write stops later rows and discards secret drafts");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<string, JsonObject, Task<JsonNode?>>? Handler;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            return Handler?.Invoke(command, arguments) ?? Task.FromResult<JsonNode?>(null);
        }
    }
}
