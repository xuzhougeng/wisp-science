using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

static class NativeSubscriptionSettingsTests
{
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    private static JsonObject Status(bool signed = false) => new() { ["signed_in"] = signed, ["account_id"] = signed ? "fixture-account" : "" };
    private static JsonObject Challenge() => new() { ["login_id"] = "fixture-attempt", ["url"] = "https://example.test/sign-in", ["method"] = "browser", ["user_code"] = "", ["message"] = "Waiting" };
    private static JsonObject Snapshot(string status) => new() { ["status"] = status, ["message"] = "", ["account_id"] = "fixture-account" };
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var state = new NativeSubscriptionSettingsModel(fake, "project-fixture");
        Check(await state.LoadAsync() && state.Statuses.Count == 3 && !state.SignedIn("codex"), "all three subscriptions load independently");
        Check(!await state.OpenAsync("codex", false) && !state.FormOpen, "signed-out subscriptions cannot create a model");
        fake.Handler = (command, args) => command == "codex_subscription_status" && args["provider"]!.GetValue<string>() == "xai" ? throw new IOException("offline fixture") : Task.FromResult<JsonNode?>(command == "list_codex_accounts" ? new JsonArray() : Status(true));
        await state.LoadAsync(); Check(state.SignedIn("chatgpt") && state.StatusErrors.ContainsKey("xai") && !state.Statuses.ContainsKey("xai"), "failed status is unknown, not signed out; other services remain usable");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(Status(true));
        await state.OpenAsync("codex", false); Check(state.CanSave && state.Model == "gpt-6.1-sol" && !state.HasChanges, "saved account creates a clean managed model draft");
        state.Set("label", "Fixture alias"); state.Set("model", " "); Check(!state.CanSave && state.HasChanges, "blank model disables save");
        state.Set("model", "fixture-model"); fake.Handler = (_, _) => throw new IOException("uncertain save fixture");
        Check(!await state.SaveAsync() && state.FormOpen && state.Label == "Fixture alias", "failed save retains the draft");
        var firstSave = fake.Calls.Last().Args.DeepClone(); fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonArray());
        Check(await state.SaveAsync() && !state.FormOpen, "confirmed model save closes the draft");
        Check(JsonNode.DeepEquals(firstSave, fake.Calls.Last().Args) && firstSave["useSaved"]!.GetValue<bool>() && !firstSave["accountOnly"]!.GetValue<bool>() && firstSave["loginId"]!.GetValue<string>().Length > 0, "explicit save retry keeps a stable creation identity and the saved-account boundary");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(Status()); await state.OpenAsync("codex", true);
        state.Set("method", "device"); Check(state.Method == "device" && !state.CanSave, "legacy subscription exposes device flow but cannot save before authorization");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(Challenge()); await state.StartAsync();
        Check(state.Pending && state.HasChanges, "challenge stays transient and activates polling");
        var count = fake.Calls.Count; Check(!await state.StartAsync() && fake.Calls.Count == count, "pending authorization is not restarted");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(Snapshot("success")); await state.PollAsync();
        Check(state.CanSave && state.LoginStatus == "success", "authorization can be explicitly saved as an account");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonArray()); await state.SaveAsync();
        var saveAccount = fake.Calls.Last(); Check(saveAccount.Args["accountOnly"]!.GetValue<bool>() && !saveAccount.Args["useSaved"]!.GetValue<bool>() && saveAccount.Args["loginId"]!.GetValue<string>() == "fixture-attempt", "account save uses only the completed login handle");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(Status()); await state.OpenAsync("chatgpt", true); state.Set("method", "device"); Check(state.Method == "browser", "official ChatGPT remains browser-only");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(Challenge()); await state.StartAsync();
        var pendingPoll = new TaskCompletionSource<JsonNode?>(); fake.Handler = (command, _) => command == "codex_login_status" ? pendingPoll.Task : Task.FromResult<JsonNode?>(null);
        var poll = state.PollAsync(); Check(await state.CloseFormAsync(), "closing cancels the owned attempt"); pendingPoll.SetResult(Snapshot("success")); await poll;
        Check(!state.FormOpen && state.LoginStatus == "idle" && state.LoginId == "", "a late poll cannot resurrect a closed form");
        Check(fake.Calls.Any(c => c.Command == "cancel_codex_login" && c.Args["loginId"]!.GetValue<string>() == "fixture-attempt"), "closed attempts reach the host cancellation command");
        var delayed = new TaskCompletionSource<JsonNode?>(); using var closing = new NativeSubscriptionSettingsModel(fake, null);
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(Status()); await closing.OpenAsync("xai", true); Check(closing.Method == "device", "SuperGrok uses device login");
        fake.Handler = (command, _) => command == "start_codex_login" ? delayed.Task : Task.FromResult<JsonNode?>(null);
        var start = closing.StartAsync(); closing.Dispose(); delayed.SetResult(Challenge()); await start;
        Check(!closing.FormOpen && fake.Calls.Last().Command == "cancel_codex_login", "challenge arriving after disposal is canceled before it can mount");
        fake.Handler = (command, _) => Task.FromResult<JsonNode?>(command == "list_codex_accounts" ? JsonNode.Parse("[{\"account_id\":\"active\",\"active\":true},{\"account_id\":\"other\",\"active\":false}]") : Status(true));
        using var accounts = new NativeSubscriptionSettingsModel(fake, "project-fixture"); await accounts.LoadAsync(); count = fake.Calls.Count;
        Check(!await accounts.AccountActionAsync("remove_codex_account", "active") && !await accounts.AccountActionAsync("remove_codex_account", "missing") && fake.Calls.Count == count, "active and unknown accounts cannot be removed");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(JsonNode.Parse("[{\"account_id\":\"other\",\"active\":true}]")); await accounts.AccountActionAsync("switch_codex_account", "other");
        Check(accounts.SignedIn("codex") && accounts.Statuses["codex"]["account_id"]!.GetValue<string>() == "other", "confirmed account switch refreshes status without profile edits");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(JsonNode.Parse("{\"imported\":1,\"accounts\":[{\"account_id\":\"imported\",\"active\":true}]}"));
        await accounts.AccountActionAsync("import_local_codex_accounts");
        Check(accounts.Imported == 1 && accounts.Accounts.Count == 1 && fake.Calls.Last().Args.Count == 0, "local import delegates discovery and credential handling entirely to the host");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(JsonNode.Parse("{\"account_id\":\"imported\",\"primary\":{\"used_percent\":10},\"secondary\":null}"));
        await accounts.RefreshUsageAsync(); Check(accounts.Usage.ContainsKey("imported"), "usage is associated with the saved account identity");
        fake.Handler = (_, _) => throw new IOException("usage fixture failure"); await accounts.RefreshUsageAsync();
        Check(accounts.Usage.Count == 0 && accounts.UsageErrors.ContainsKey("imported"), "failed usage clears stale percentages and displays an error");
        Check(fake.Calls.Where(c => c.Command is "save_codex_login" or "switch_codex_account").All(c => c.Project == "project-fixture"), "writes retain project binding");
        Check(NativeSubscriptionSettingsModel.IsSubscription(new() { ["provider"] = "openai-chatgpt" }) && !NativeSubscriptionSettingsModel.IsSubscription(new() { ["provider"] = "openai" }), "classification uses normalized protocol identities");
        var profile = new JsonObject { ["id"] = "managed-fixture", ["provider"] = "openai_chatgpt", ["model"] = "gpt-6.1-sol", ["api_url"] = "https://example.test/managed", ["max_tokens"] = 0, ["label"] = "Original", ["use_for_expert"] = false };
        var editorClient = new Fake { Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonArray()) };
        using var editor = new NativeSettingsEditorModel(editorClient, "project-fixture"); editor.Edit(profile, "save_model", "profile");
        editor.Draft!["label"] = "Alias"; editor.Draft["use_for_expert"] = true; await editor.SaveAsync();
        var written = editorClient.Calls.Last().Args["profile"]!;
        Check(written["provider"]!.GetValue<string>() == "openai_chatgpt" && written["api_url"]!.GetValue<string>() == "https://example.test/managed" && written["max_tokens"]!.GetValue<int>() == 0 && written["use_for_expert"]!.GetValue<bool>(), "subscription editor preserves managed connection and automatic limits while saving alias and expert role");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<string, JsonObject, Task<JsonNode?>>? Handler;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            return Handler?.Invoke(command, arguments) ?? Task.FromResult<JsonNode?>(command == "list_codex_accounts" ? new JsonArray() : Status());
        }
    }
}
