using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// A mounted, project-bound settings editor. Only confirmed writes discard its draft.
public sealed partial class NativeSettingsEditorModel(INativeSettingsClient client, string? projectId, string? sessionId = null) : WorkspaceActionModel
{
    public JsonObject Values { get; private set; } = new();
    public JsonObject? Draft { get; private set; }
    private JsonObject? original;
    private string? command, parameter;
    // Reuse the same identity if the host saved a new agent but its reply was lost.
    private string? newAcpId;
    private JsonObject extra = new();
    private NativeSettingsWrite[]? preferenceWrites;
    private NativeWindowPreferences? windowPreferences;
    public bool EditingWindowPreferences => windowPreferences != null;
    public bool InlinePreferences => preferenceWrites != null;
    public string? HostVersion => Values["get_bootstrap_status"] is JsonObject bootstrap
        && bootstrap["app_version"] is JsonValue value && value.TryGetValue<string>(out var version)
        && !string.IsNullOrWhiteSpace(version) ? version : null;
    public bool HasChanges => Draft != null && !JsonNode.DeepEquals(original, Draft);
    public bool OAuthPending { get; private set; }
    public bool Saving { get; private set; }
    public bool SaveConfirmed { get; private set; }
    private bool cancellingOAuth;
    private long catalogRequest;
    public NativeLocalization? Localization { private get; set; }
    public bool LoadFailed { get; private set; }
    public string ReadFailure(Exception ex) => (Localization?.Choose("未能读取设置。", "Could not load settings.") ?? "未能读取设置。") + "\n" + ex.Message;
    public Task<bool> LoadAsync(params string[] commands) => LoadAsync(commands, CancellationToken.None);
    public Task<bool> LoadAsync(string[] commands, CancellationToken token) => LoadSnapshotAsync(commands, null, token);
    public Task<bool> RetryLoadAsync(string[] commands, Func<Task> reconnect, CancellationToken token = default) =>
        LoadFailed ? LoadSnapshotAsync(commands, reconnect, token) : Task.FromResult(false);
    private Task<bool> LoadSnapshotAsync(string[] commands, Func<Task>? reconnect, CancellationToken token) => RunAsync(async () =>
    {
        if (reconnect != null) await reconnect();
        var loaded = new JsonObject();
        foreach (var read in commands)
        {
            if (Closed) break;
            var value = await client.InvokeAsync(read, new(), projectId, token);
            loaded[read] = read == "get_settings" && value is JsonObject prefs
                ? NativeWindowPreferences.EditableSnapshot(prefs) : value?.DeepClone();
        }
        return loaded;
    }, loaded => { Values = loaded; LoadFailed = false; }, ex => { LoadFailed = true; return ReadFailure(ex); });

    // Optional reads stay separate: an old host or a failed catalog request must
    // not prevent editing the already loaded general preferences.
    public Task<bool> LoadNotificationSoundsAsync() => RunAsync(async () =>
    {
        var capabilities = await client.InvokeAsync("native_settings_capabilities", new(), projectId);
        if (Closed || !NativeNotificationSounds.Supported(capabilities)) return null;
        return await client.InvokeAsync("list_notification_sounds", new(), projectId);
    }, catalog => Values["list_notification_sounds"] = catalog?.DeepClone(), ReadFailure);

    public Task<bool> LoadChannelCapabilitiesAsync() => RunAsync(
        () => client.InvokeAsync("native_settings_capabilities", new(), projectId),
        value => Values["native_settings_capabilities"] = value?.DeepClone(), ReadFailure);

    public Task<bool> SetSubagentModelAsync(string id)
    {
        if (Busy || Closed || Draft != null || Values["get_subagent_model"] is not JsonValue value
            || !value.TryGetValue<string>(out var current)) return Task.FromResult(false);
        if (id == current) return Task.FromResult(true);
        if (!NativeSpecialists.SubagentOptions(current, (Values["list_models"] as JsonArray)?.OfType<JsonObject>() ?? [], Localization)
            .Any(option => option.Id == id && option.Available)) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            var reply = await client.InvokeAsync("set_subagent_model", new() { ["id"] = id }, projectId);
            if (reply is not JsonValue acknowledged || !acknowledged.TryGetValue<string>(out var saved) || saved != id)
                throw new InvalidDataException(Localization?.Choose("未能确认子智能体模型设置。", "Could not confirm the subagent model setting.")
                    ?? "Could not confirm the subagent model setting.");
            return saved;
        }, saved => Values["get_subagent_model"] = saved,
            ex => (Localization?.Choose("未能保存子智能体模型设置。", "Could not save the subagent model setting.")
                ?? "Could not save the subagent model setting.") + "\n" + ex.Message);
    }

    public Task<bool> SetUpdateCheckEnabledAsync(bool enabled)
    {
        if (Busy || Closed || Values["get_update_check_enabled"] is not JsonValue value
            || !value.TryGetValue<bool>(out var current)) return Task.FromResult(false);
        if (enabled == current) return Task.FromResult(true);
        // WebView commits this independent preference immediately. A later
        // General Save/Cancel must neither replay nor undo the acknowledged write.
        return RunAsync(async () =>
        {
            var reply = await client.InvokeAsync("set_update_check_enabled", new() { ["enabled"] = enabled }, projectId);
            if (reply is not JsonValue acknowledged || !acknowledged.TryGetValue<bool>(out var saved) || saved != enabled)
                throw new InvalidDataException("Could not confirm the update preference.");
            return saved;
        }, _ =>
        {
            Values["get_update_check_enabled"] = enabled;
            if (Draft?.ContainsKey("get_update_check_enabled") == true)
            {
                Draft["get_update_check_enabled"] = enabled;
                original!["get_update_check_enabled"] = enabled;
            }
        });
    }

    public Task<bool> PreviewNotificationSoundAsync()
    {
        var prefs = InlinePreferences ? Draft?["get_settings"] as JsonObject : Draft;
        if (prefs == null || !InlinePreferences && command != "set_settings" || Busy || Closed
            || !prefs.ContainsKey("notification_sound_enabled")) return Task.FromResult(false);
        var sound = prefs["notification_sound"]?.GetValue<string>() ?? "";
        if (!NativeNotificationSounds.CanPreview(Values["list_notification_sounds"], sound)) return Task.FromResult(false);
        return InvokeAsync("preview_notification_sound", new() { ["sound"] = sound });
    }

    public void Edit(JsonObject draft, string write, string? argument = null, JsonObject? arguments = null)
    {
        if (Busy || Closed) return;
        newAcpId = null;
        if (write == "save_acp_agent") ClearFailure();
        preferenceWrites = null;
        windowPreferences = null;
        SaveConfirmed = false;
        ResetReviewerTest();
        ResetProviderBatch();
        original = (JsonObject)draft.DeepClone(); Draft = (JsonObject)draft.DeepClone();
        command = write; parameter = argument; extra = (JsonObject?)arguments?.DeepClone() ?? new();
    }
    public void EditPreferences(params NativeSettingsWrite[] writes)
    {
        if (Busy || Closed || Draft != null) return;
        var available = writes.Where(w => Values.ContainsKey(w.Read)).ToArray();
        var draft = new JsonObject();
        foreach (var write in available) draft[write.Read] = Values[write.Read]?.DeepClone();
        Edit(draft, "preferences");
        preferenceWrites = available;
    }
    public void EditWindowPreferences(NativeWindowPreferences preferences)
    {
        if (Busy || Closed || Draft != null || Values["get_settings"] is not JsonObject) return;
        EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        windowPreferences = preferences;
        preferences.ApplyTo(Draft!["get_settings"]!.AsObject());
    }
    public void KeepWindowPreferences()
    {
        if (original?["get_settings"] is JsonObject baseline && Draft?["get_settings"] is JsonObject form)
            windowPreferences?.Capture(baseline, form);
    }
    public void Discard() { if (!Busy) { if (command == "save_acp_agent") ClearFailure(); newAcpId = null; Draft = null; original = null; command = null; preferenceWrites = null; windowPreferences = null; SaveConfirmed = false; ResetReviewerTest(); ResetProviderBatch(); } }
    public bool CancelPreferences()
    {
        if (Busy || Closed || !InlinePreferences || Draft == null) return false;
        windowPreferences?.Clear();
        Discard();
        return true;
    }
    public async Task<bool> SaveAsync()
    {
        if (Busy || Closed || Draft == null || command == null) return false;
        if (AddingProvider) return await SaveProviderAsync();
        if (command is "save_model" or "add_mcp_connection" or "update_mcp_connection" && NativeProxyRoutes.ErrorKey(Draft) is { } routeError)
        { Fail(routeError); return false; }
        if (command == "save_quick_action" && NativeQuickActions.SaveBlockReason(Draft) is { } error) { Fail(error); return false; }
        if (command == "save_acp_agent" && !NativeModelDrafts.AcpValid(Draft)) { Fail(Localization?.Text("models.acp_required") ?? "Label and command are required"); return false; }
        if (command == "save_specialist_cmd" && NativeSpecialists.SaveBlockReason(Draft) is { } specialistError) { Fail(specialistError); return false; }
        if (command == "set_device_bridge" && NativeChannelSettings.DeviceSaveBlockReason(Draft) is { } deviceError) { Fail(deviceError); return false; }
        if (preferenceWrites != null) return await SavePreferencesAsync();
        var args = (JsonObject)extra.DeepClone();
        if (command == "install_plugin" && string.IsNullOrWhiteSpace(Draft["expected_sha256"]?.GetValue<string>())) Draft["expected_sha256"] = null;
        if (command == "save_model")
            foreach (var field in NativeModelDrafts.SaveArguments(Draft)) args[field.Key] = field.Value?.DeepClone();
        else if (parameter != null) args[parameter] = command == "save_specialist_cmd" ? NativeSpecialists.SaveDraft(Draft, original!)
            : command == "save_acp_agent" ? NativeModelDrafts.AcpSaveDraft(Draft)
            : command is "add_mcp_connection" or "update_mcp_connection" ? NativeConnections.SaveDraft(Draft) : Draft.DeepClone();
        else foreach (var field in command == "set_device_bridge" ? NativeChannelSettings.DeviceSaveDraft(Draft) : Draft) args[Camel(field.Key)] = field.Value?.DeepClone();
        var operation = command is "add_mcp_connection" or "update_mcp_connection" && Draft["transport"]?["auth"]?.GetValue<string>() == "oauth"
            ? "authorize_http_connection" : command;
        OAuthPending = operation == "authorize_http_connection";
        Saving = true; SaveConfirmed = false;
        try
        {
            var agent = operation == "save_acp_agent" ? args["profile"]!.AsObject() : null;
            if (agent != null && string.IsNullOrWhiteSpace(agent["id"]?.GetValue<string>())) agent["id"] = newAcpId ??= Guid.NewGuid().ToString();
            return await RunAsync(() => client.InvokeAsync(operation, args, projectId), reply =>
            {
                if (agent != null)
                {
                    if (NativeModelDrafts.ConfirmedAcpList(reply, agent) is not { } list)
                        throw new InvalidDataException(Localization?.Choose("宿主未返回已保存的代理配置。", "The host did not return the saved agent profile.") ?? "The host did not return the saved agent profile.");
                    Values["list_acp_agents"] = list.DeepClone();
                }
                newAcpId = null; Draft = null; original = null; command = null; SaveConfirmed = true;
            });
        }
        finally { OAuthPending = false; Saving = false; if (!Closed) Notify(); }
    }
    private async Task<bool> SavePreferencesAsync()
    {
        var submitted = (JsonObject)Draft!.DeepClone();
        var writes = preferenceWrites!.Where(w => !JsonNode.DeepEquals(original![w.Read], submitted[w.Read])).ToArray();
        KeepWindowPreferences();
        Saving = true; SaveConfirmed = false;
        try
        {
            return await RunAsync(async () =>
            {
                foreach (var write in writes)
                {
                    if (windowPreferences != null && write.Read == "get_settings")
                    {
                        // set_settings accepts a full document. Do not roll back
                        // unrelated changes made since this section was opened.
                        var latest = await client.InvokeAsync(write.Read, new(), projectId) as JsonObject
                            ?? throw new InvalidDataException("Could not read the current settings.");
                        if (Closed) return false;
                        var merged = NativeWindowPreferences.EditableSnapshot(latest);
                        windowPreferences.ApplyTo(merged);
                        submitted[write.Read] = merged;
                    }
                    await client.InvokeAsync(write.Command, new() { [write.Parameter] = submitted[write.Read]?.DeepClone() }, projectId);
                    if (Closed) return false;
                    if (windowPreferences != null && submitted[write.Read] is JsonObject applied
                        && !string.IsNullOrWhiteSpace(applied["sync_relay_token"]?.GetValue<string>()))
                    {
                        applied["sync_relay_token"] = "";
                        applied["has_sync_relay_token"] = true;
                    }
                    // A later failure must not replay writes the host already acknowledged.
                    original![write.Read] = submitted[write.Read]?.DeepClone();
                    Values[write.Read] = submitted[write.Read]?.DeepClone();
                }
                return true;
            }, _ => { windowPreferences?.Clear(); windowPreferences = null; Draft = null; original = null; command = null; preferenceWrites = null; SaveConfirmed = true; });
        }
        finally { Saving = false; if (!Closed) Notify(); }
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
        if (AddingProvider) return TestProviderAsync(apply);
        if (Draft == null || command != "save_model" || Busy || Closed) return Task.FromResult(false);
        if (NativeProxyRoutes.ErrorKey(Draft) is { } routeError) { Fail(routeError); return Task.FromResult(false); }
        var draft = (JsonObject)Draft.DeepClone();
        var proxyUrl = NativeSubscriptionSettingsModel.IsSubscription(draft) ? null
            : NativeProxyRoutes.ModelValue(draft, (Values["list_models"] as JsonArray)?.OfType<JsonObject>() ?? []);
        return RunAsync(async () =>
        {
            var settings = await client.InvokeAsync("get_settings", new(), projectId) as JsonObject
                ?? throw new InvalidDataException("无法读取当前设置。");
            if (Closed) return null;
            return await client.InvokeAsync("validate_settings", NativeModelDrafts.TestArguments(settings, draft, proxyUrl), projectId);
        }, apply);
    }
    // Catalog lookup reads the host's baked exact-ID metadata. It never tests an
    // endpoint or persists a draft. Obsolete replies cannot alter another form.
    public async Task ReadModelCatalogAsync(Action<JsonObject?> apply)
    {
        var request = ++catalogRequest;
        if (Draft is not { } draft || command != "save_model" || Busy || Closed) return;
        var args = NativeModelDrafts.CatalogArguments(draft);
        apply(null);
        if (string.IsNullOrWhiteSpace(args["model"]?.GetValue<string>()) || !NativeModelPresentation.IsChat(draft)) return;
        JsonObject? result;
        try { result = await client.InvokeAsync("model_catalog_lookup", args, projectId) as JsonObject; }
        catch { result = null; }
        if (request == catalogRequest && !Closed && !Busy && ReferenceEquals(Draft, draft)
            && JsonNode.DeepEquals(args, NativeModelDrafts.CatalogArguments(draft))) apply(result);
    }
    public override void Dispose() { base.Dispose(); ClearEnvironmentEditor(); DiscardCredentialInputs(); Draft = null; original = null; windowPreferences = null; Values.Clear(); ResetReviewerTest(); ResetProviderBatch(); }
    private static string Camel(string key)
    {
        var words = key.Split('_');
        return words[0] + string.Concat(words.Skip(1).Select(w => w.Length == 0 ? "" : char.ToUpperInvariant(w[0]) + w[1..]));
    }
}

public sealed record NativeSettingsWrite(string Read, string Command, string Parameter);
