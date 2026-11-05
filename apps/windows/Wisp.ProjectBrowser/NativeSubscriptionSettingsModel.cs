using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Subscription metadata and one mounted sign-in/model draft. Tokens stay in the host.
public sealed class NativeSubscriptionSettingsModel(INativeSettingsClient client, string? projectId) : WorkspaceActionModel
{
    public static readonly (string Id, string Title, string Description, string Profile)[] Providers =
    [ ("chatgpt", "ChatGPT", "subscriptions.chatgpt_signin", "openai_chatgpt"),
      ("codex", "ChatGPT Codex (legacy)", "subscriptions.chatgpt", "openai_codex"),
      ("xai", "SuperGrok", "subscriptions.xai", "xai_oauth") ];
    public Dictionary<string, JsonObject> Statuses { get; } = [];
    public Dictionary<string, string> StatusErrors { get; } = [];
    public JsonArray Accounts { get; private set; } = [];
    public Dictionary<string, JsonObject> Usage { get; } = [];
    public Dictionary<string, string> UsageErrors { get; } = [];
    public string? AccountsError { get; private set; }
    public int? Imported { get; private set; }
    public string? Provider { get; private set; }
    public bool AccountOnly { get; private set; }
    public string Method { get; private set; } = "browser";
    public string Model { get; private set; } = "";
    public string Label { get; private set; } = "";
    public string Redirect { get; private set; } = "";
    public string SavedAccount { get; private set; } = "";
    public string LoginId { get; private set; } = "";
    public string LoginStatus { get; private set; } = "idle";
    public string LoginMessage { get; private set; } = "";
    public string LoginUrl { get; private set; } = "";
    public string UserCode { get; private set; } = "";
    public JsonArray? SavedModels { get; private set; }
    public bool Pending => LoginStatus == "pending";
    public bool FormOpen => Provider != null;
    public bool CanSave => !Closed && !Busy && FormOpen && (AccountOnly ? LoginStatus == "success" : Model.Trim().Length > 0 && SavedAccount.Length > 0);
    private string originalModel = "", originalLabel = "", creationId = "";
    private int formGeneration;
    private bool polling;
    public bool HasChanges => FormOpen && (!AccountOnly && (Model != originalModel || Label != originalLabel) || LoginId.Length > 0 || Redirect.Length > 0);
    public static string Text(JsonNode? node, string key) => node?[key]?.GetValue<string>() ?? "";
    public bool SignedIn(string provider) => Statuses.TryGetValue(provider, out var status) && status["signed_in"]?.GetValue<bool>() == true;
    public static bool IsSubscription(JsonObject profile) => Providers.Any(p => p.Profile == NativeModelDrafts.ProviderValue(Text(profile, "provider")));
    public static string LoginKey(string provider, string key) => provider switch
    {
        "chatgpt" when key is "codex.login.desc" or "codex.login.label_ph" or "codex.login.browser_hint" or "codex.login.paste" or "codex.login.paste_ph" => key.Replace("codex.", "chatgpt."),
        "xai" when key is "codex.login.title" or "codex.login.desc" or "codex.login.label_ph" or "codex.login.device_hint" or "codex.login.use_saved" or "codex.login.waiting" or "codex.login.saved_hint" => key.Replace("codex.", "xai."),
        _ => key
    };
    public Task<bool> LoadAsync() => RunAsync(async () =>
    {
        var statuses = new Dictionary<string, JsonObject>(); var errors = new Dictionary<string, string>();
        foreach (var provider in Providers)
        {
            if (Closed) break;
            try { statuses[provider.Id] = (await client.InvokeAsync("codex_subscription_status", new() { ["provider"] = provider.Id }, projectId))?.AsObject() ?? throw new InvalidDataException("Missing account status."); }
            catch (Exception error) { errors[provider.Id] = error.Message; }
        }
        JsonArray accounts = []; string? accountError = null;
        if (!Closed) try { accounts = (await client.InvokeAsync("list_codex_accounts", new(), projectId))?.AsArray() ?? throw new InvalidDataException("Missing account list."); }
        catch (Exception error) { accountError = error.Message; }
        return (statuses, errors, accounts, accountError);
    }, result =>
    {
        Statuses.Clear(); StatusErrors.Clear(); Usage.Clear(); UsageErrors.Clear(); Imported = null;
        foreach (var item in result.statuses) Statuses[item.Key] = (JsonObject)item.Value.DeepClone();
        foreach (var item in result.errors) StatusErrors[item.Key] = item.Value;
        Accounts = (JsonArray)result.accounts.DeepClone(); AccountsError = result.accountError;
    });
    public Task<bool> RefreshUsageAsync() => RunAsync(async () =>
    {
        var usage = new Dictionary<string, JsonObject>(); var errors = new Dictionary<string, string>();
        foreach (var account in Accounts.OfType<JsonObject>())
        {
            if (Closed) break;
            var id = Text(account, "account_id");
            try { usage[id] = (await client.InvokeAsync("codex_account_usage", new() { ["accountId"] = id }, projectId))?.AsObject() ?? throw new InvalidDataException("Missing account usage."); }
            catch (Exception error) { errors[id] = error.Message; }
        }
        return (usage, errors);
    }, result => { Usage.Clear(); UsageErrors.Clear(); foreach (var item in result.usage) Usage[item.Key] = item.Value; foreach (var item in result.errors) UsageErrors[item.Key] = item.Value; });
    public Task<bool> AccountActionAsync(string command, string? accountId = null)
    {
        if (FormOpen || command is not ("switch_codex_account" or "remove_codex_account" or "import_local_codex_accounts")) return Task.FromResult(false);
        if (command != "import_local_codex_accounts" && !Accounts.OfType<JsonObject>().Any(a => Text(a, "account_id") == accountId && a["active"]?.GetValue<bool>() != true)) return Task.FromResult(false);
        return RunAsync(() => client.InvokeAsync(command, accountId == null ? new() : new() { ["accountId"] = accountId }, projectId), result =>
        {
            Accounts = (JsonArray)(command == "import_local_codex_accounts" ? result!["accounts"]! : result!).DeepClone();
            Imported = command == "import_local_codex_accounts" ? result!["imported"]!.GetValue<int>() : null;
            AccountsError = null; Usage.Clear(); UsageErrors.Clear();
            var active = Accounts.OfType<JsonObject>().FirstOrDefault(a => a["active"]?.GetValue<bool>() == true);
            Statuses["codex"] = new() { ["signed_in"] = active != null, ["account_id"] = Text(active, "account_id") }; StatusErrors.Remove("codex");
        });
    }
    public async Task<bool> OpenAsync(string provider, bool accountOnly)
    {
        if (Closed || Busy || FormOpen || !Providers.Any(p => p.Id == provider) || !accountOnly && !SignedIn(provider)) return false;
        formGeneration++; Provider = provider; AccountOnly = accountOnly; Method = provider == "xai" ? "device" : "browser";
        Model = originalModel = provider == "xai" ? "grok-4.6" : "gpt-6.1-sol"; Label = originalLabel = "";
        creationId = Guid.NewGuid().ToString("N"); SavedAccount = ""; SavedModels = null; ResetLogin();
        return await RunAsync(() => client.InvokeAsync("codex_subscription_status", new() { ["provider"] = provider }, projectId), value =>
        { if (value?["signed_in"]?.GetValue<bool>() == true) SavedAccount = Text(value, "account_id"); });
    }
    public void Set(string field, string value)
    {
        if (Busy || Closed || !FormOpen) return;
        switch (field)
        {
            case "method" when Provider == "codex" && !Pending && value is "browser" or "device": Method = value; break;
            case "model": Model = value; break;
            case "label": Label = value; break;
            case "redirect": Redirect = value; break;
        }
        ClearFailure(); Notify();
    }
    private void ResetLogin() { LoginId = ""; LoginStatus = "idle"; LoginMessage = ""; LoginUrl = ""; UserCode = ""; Redirect = ""; ClearFailure(); }
    private async Task CancelRemote(string id) { if (id.Length > 0) await client.InvokeAsync("cancel_codex_login", new() { ["loginId"] = id }, projectId); }
    public async Task<bool> CancelAttemptAsync()
    {
        if (Busy || Closed) return false;
        formGeneration++;
        return await RunAsync(async () => { await CancelRemote(LoginId); return true; }, _ => ResetLogin());
    }
    public async Task<bool> CloseFormAsync()
    {
        if (Busy || Closed) return false;
        if (!await CancelAttemptAsync()) return false;
        Provider = null; SavedAccount = ""; Model = ""; Label = ""; Notify(); return true;
    }
    public async Task<bool> StartAsync()
    {
        if (!FormOpen || !AccountOnly || Busy || Closed || Pending) return false;
        if (LoginId.Length > 0 && !await CancelAttemptAsync()) return false;
        var version = ++formGeneration; var provider = Provider!; var method = Method;
        return await RunAsync(async () =>
        {
            var challenge = (await client.InvokeAsync("start_codex_login", new() { ["provider"] = provider, ["method"] = method }, projectId))?.AsObject() ?? throw new InvalidDataException("Missing sign-in challenge.");
            if (Closed || formGeneration != version) { await CancelRemote(Text(challenge, "login_id")); return null; }
            if (Text(challenge, "login_id").Length == 0) throw new InvalidDataException("Missing sign-in identifier.");
            return challenge;
        }, challenge =>
        {
            if (challenge == null) return;
            LoginId = Text(challenge, "login_id"); LoginUrl = Text(challenge, "url"); UserCode = Text(challenge, "user_code");
            LoginMessage = Text(challenge, "message"); LoginStatus = "pending";
        });
    }
    private void ApplySnapshot(JsonObject snapshot)
    {
        LoginStatus = Text(snapshot, "status"); LoginMessage = Text(snapshot, "message");
        if (LoginStatus == "success") Redirect = "";
    }
    public async Task<bool> PollAsync()
    {
        if (!Pending || LoginId.Length == 0 || Busy || Closed || polling) return false;
        var version = formGeneration; var id = LoginId; polling = true;
        try
        {
            var value = (await client.InvokeAsync("codex_login_status", new() { ["loginId"] = id }, projectId))?.AsObject() ?? throw new InvalidDataException("Missing sign-in status.");
            if (Closed || version != formGeneration || id != LoginId || Busy) return false;
            if (LoginStatus == Text(value, "status") && LoginMessage == Text(value, "message")) return true;
            ApplySnapshot(value); Notify(); return true;
        }
        catch (Exception error) { if (!Closed && version == formGeneration && id == LoginId) { LoginStatus = "error"; LoginMessage = error.Message; Notify(); } return false; }
        finally { polling = false; }
    }
    public Task<bool> SubmitRedirectAsync()
    {
        if (!FormOpen || Method != "browser" || LoginId.Length == 0 || string.IsNullOrWhiteSpace(Redirect)) return Task.FromResult(false);
        formGeneration++;
        return RunAsync(async () => (await client.InvokeAsync("submit_codex_login_redirect", new() { ["loginId"] = LoginId, ["redirect"] = Redirect }, projectId))?.AsObject() ?? throw new InvalidDataException("Missing sign-in status."), ApplySnapshot);
    }
    public Task<bool> SaveAsync()
    {
        if (!CanSave) return Task.FromResult(false);
        var args = new JsonObject { ["loginId"] = AccountOnly ? LoginId : creationId, ["provider"] = Provider,
            ["model"] = Model.Trim(), ["label"] = Label.Trim(), ["profileId"] = "", ["accountOnly"] = AccountOnly, ["useSaved"] = !AccountOnly };
        return RunAsync(async () => (await client.InvokeAsync("save_codex_login", args, projectId))?.AsArray() ?? throw new InvalidDataException("Missing saved model list."), value =>
        { SavedModels = (JsonArray)value.DeepClone(); formGeneration++; ResetLogin(); Provider = null; SavedAccount = ""; });
    }
    public override void Dispose()
    {
        var id = LoginId; base.Dispose(); formGeneration++; Provider = null; Redirect = ""; LoginId = ""; LoginUrl = ""; UserCode = "";
        if (id.Length > 0) _ = CancelAfterClose(id);
    }
    private async Task CancelAfterClose(string id) { try { await CancelRemote(id); } catch { /* The closed page cannot apply or retry this response. */ } }
}
