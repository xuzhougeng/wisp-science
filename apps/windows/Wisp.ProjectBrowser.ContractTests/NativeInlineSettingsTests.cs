using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeInlineSettingsTests
{
    public static async Task RunAsync()
    {
        await SharedWindowDraftAsync();
        await SyncWindowDraftAsync();
        var client = new Fake();
        using var model = new NativeSettingsEditorModel(client, "project-a");
        Check(model.HostVersion == null, "no version is invented before the host responds");
        await model.LoadAsync("get_settings", "get_appearance_prefs", "get_update_check_enabled", "get_bootstrap_status");
        Check(model.HostVersion == "1.22.0", "General displays the actual host version");
        foreach (var invalid in new JsonNode?[] { null, JsonValue.Create(22), JsonValue.Create(" "), new JsonObject() })
        {
            model.Values["get_bootstrap_status"]!["app_version"] = invalid;
            Check(model.HostVersion == null, "unsupported version values do not crash the settings page");
        }
        var writes = new NativeSettingsWrite[] { new("get_settings", "set_settings", "settings"),
            new("get_appearance_prefs", "set_appearance_prefs", "prefs"), new("get_update_check_enabled", "set_update_check_enabled", "enabled") };
        model.EditPreferences(writes);
        Check(model.InlinePreferences && !model.HasChanges, "opening direct fields does not change settings");
        Check(await model.SaveAsync() && model.SaveConfirmed, "unchanged preferences can finish Save without a mutation");
        Check(client.Writes.Count == 0, "unchanged categories do not submit writes");
        model.EditPreferences(writes);
        model.Draft!["get_settings"]!["locale"] = "zh";
        model.Draft["get_appearance_prefs"]!["send_with_modifier"] = true;
        var languageDraft = model.Draft;
        model.EditPreferences(writes); // Rebuilding translated controls must reuse the open draft.
        Check(ReferenceEquals(languageDraft, model.Draft) && model.Draft["get_settings"]!["locale"]!.GetValue<string>() == "zh"
            && model.Draft["get_appearance_prefs"]!["send_with_modifier"]!.GetValue<bool>(),
            "language preview rerender preserves all in-progress preference edits");
        Check(client.Writes.Count == 0, "previewing and rebuilding settings never submits a preference write");
        client.FailAppearance = true;
        Check(!await model.SaveAsync() && model.HasChanges && !model.SaveConfirmed, "partial failure retains the remaining draft");
        Check(client.Writes.Count == 2 && client.Writes[0].Args["settings"]!["future_flag"]!.GetValue<bool>(), "full settings preserve unknown fields");
        client.FailAppearance = false;
        await model.SaveAsync();
        Check(client.Writes.Count == 3 && client.Writes[2].Command == "set_appearance_prefs", "retry never repeats already acknowledged writes");
        Check(client.Writes[2].Args["prefs"]!["theme"]!.GetValue<string>() == "system", "shortcut edits preserve unrelated appearance preferences");
        Check(model.Draft == null && model.SaveConfirmed && !model.Busy, "only fully confirmed saves clear the form draft");
        model.EditPreferences(writes);
        model.Draft!["get_update_check_enabled"] = false;
        await model.SaveAsync();
        Check(client.Writes[^1].Command == "set_update_check_enabled" && !client.Writes[^1].Args["enabled"]!.GetValue<bool>(), "scalar settings retain the backend argument shape");
        var beforeCancel = client.Writes.Count;
        model.EditPreferences(writes); model.Draft!["get_settings"]!["locale"] = "en";
        model.Draft["get_appearance_prefs"]!["send_with_modifier"] = false; model.Discard();
        Check(model.Values["get_settings"]!["locale"]!.GetValue<string>() == "zh"
            && model.Values["get_appearance_prefs"]!["send_with_modifier"]!.GetValue<bool>()
            && client.Writes.Count == beforeCancel && model.Draft == null && !model.HasChanges,
            "General cancel discards all preference drafts without changing confirmed values or writing");
        model.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        model.Draft!["get_settings"]!["semantic_compact_on_model_switch"] = true;
        model.Draft["get_settings"]!["show_tips"] = false;
        var beforeSessionSave = client.Writes.Count;
        Check(await model.SaveAsync() && client.Writes.Count == beforeSessionSave + 1
            && client.Writes[^1].Command == "set_settings", "session edits write only their existing settings contract");
        var session = client.Writes[^1].Args["settings"]!;
        Check(session["semantic_compact_on_model_switch"]!.GetValue<bool>() && !session["show_tips"]!.GetValue<bool>()
            && session["auto_continue_limit"]!.GetValue<int>() == 10 && session["future_flag"]!.GetValue<bool>(),
            "new session switches preserve disabled dependent values and unrelated preferences");
        model.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        model.Draft!["get_settings"]!["show_tips"] = true; model.Discard();
        Check(!model.Values["get_settings"]!["show_tips"]!.GetValue<bool>() && client.Writes.Count == beforeSessionSave + 1,
            "session cancel restores the saved switches without a write");
        await ImmediateUpdateCheckAsync();
        await CancelPreferencesAsync();
    }
    private static async Task CancelPreferencesAsync()
    {
        var client = new Fake();
        using var model = new NativeSettingsEditorModel(client, "project-a");
        Check(!model.CancelPreferences(), "Cancel cannot close an unloaded preference editor");
        await model.LoadAsync("get_settings");
        model.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        Check(model.CancelPreferences() && client.Writes.Count == 0, "unchanged preferences can be cancelled without a write");
        model.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        model.Draft!["get_settings"]!["auto_continue"] = true;
        model.Draft["get_settings"]!["pet_enabled"] = true;
        model.Draft["get_settings"]!["pet_directory"] = "draft-only-pet";
        Check(model.CancelPreferences() && model.Draft == null && !model.InlinePreferences && !model.HasChanges
            && !model.Values["get_settings"]!["auto_continue"]!.GetValue<bool>()
            && model.Values["get_settings"]!["pet_directory"] == null && client.Writes.Count == 0,
            "Cancel closes Session/Pet drafts and preserves the saved preferences without submitting anything");
        model.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        model.Draft!["get_settings"]!["auto_continue"] = true;
        client.SettingsReply = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var save = model.SaveAsync();
        Check(model.Busy && !model.CancelPreferences() && model.Draft != null,
            "Cancel cannot dismiss preferences while their write awaits acknowledgment");
        client.SettingsReply.SetResult(null);
        Check(await save && !model.CancelPreferences(), "a completed save clears the previous cancel target");
        model.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        model.Dispose();
        Check(!model.CancelPreferences(), "a stale Cancel callback cannot affect a closed surface");
    }
    private static async Task ImmediateUpdateCheckAsync()
    {
        var client = new Fake();
        using var model = new NativeSettingsEditorModel(client, "project-a");
        Check(!await model.SetUpdateCheckEnabledAsync(false) && client.Writes.Count == 0, "unloaded update preferences cannot be changed");
        await model.LoadAsync("get_settings", "get_appearance_prefs", "get_update_check_enabled");
        var writes = new NativeSettingsWrite[] { new("get_settings", "set_settings", "settings"), new("get_appearance_prefs", "set_appearance_prefs", "prefs") };
        model.EditPreferences(writes); model.Draft!["get_settings"]!["locale"] = "zh";
        var draft = model.Draft;
        Check(await model.SetUpdateCheckEnabledAsync(false) && client.Writes.Count == 1
            && client.Writes[0].Command == "set_update_check_enabled" && !client.Writes[0].Args["enabled"]!.GetValue<bool>(),
            "update-check commits its bool immediately through the existing host command");
        Check(ReferenceEquals(draft, model.Draft) && model.HasChanges && model.Draft!["get_settings"]!["locale"]!.GetValue<string>() == "zh",
            "immediate update preference preserves unrelated General drafts");
        model.Discard(); model.EditPreferences(writes);
        Check(!model.Values["get_update_check_enabled"]!.GetValue<bool>() && !model.HasChanges && client.Writes.Count == 1,
            "General cancel never undoes the independently committed update setting");
        Check(await model.SetUpdateCheckEnabledAsync(false) && client.Writes.Count == 1, "same-value updates do not write again");
        Check(await model.SaveAsync() && client.Writes.Count == 1, "General Save does not replay an immediate update write");

        client.FailUpdate = true;
        Check(!await model.SetUpdateCheckEnabledAsync(true) && !model.Values["get_update_check_enabled"]!.GetValue<bool>()
            && model.Error != null && client.Writes.Count == 2, "unconfirmed writes retain the confirmed value and surface an error without retrying");
        client.FailUpdate = false; client.UpdateReply = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var pending = model.SetUpdateCheckEnabledAsync(true);
        Check(model.Busy && !await model.SetUpdateCheckEnabledAsync(false) && client.Writes.Count == 3,
            "competing toggles cannot reorder a pending preference write");
        client.UpdateReply.SetResult(JsonValue.Create(true));
        Check(await pending && model.Values["get_update_check_enabled"]!.GetValue<bool>() && model.Error == null,
            "an explicit successful retry updates the displayed value");

        client.UpdateReply = new(TaskCreationOptions.RunContinuationsAsynchronously);
        pending = model.SetUpdateCheckEnabledAsync(false); model.Dispose();
        Check(model.Values.Count == 0, "closing the editor clears its settings snapshot");
        client.UpdateReply.SetResult(JsonValue.Create(false));
        Check(!await pending && model.Values.Count == 0, "a successful late reply cannot repopulate a closed settings surface");
    }
    private static async Task SharedWindowDraftAsync()
    {
        var host = new Fake();
        var window = new NativeWindowPreferences();
        async Task<NativeSettingsEditorModel> Mount(NativeWindowPreferences scope)
        {
            var pane = new NativeSettingsEditorModel(host, "project-a");
            await pane.LoadAsync("get_settings");
            pane.EditWindowPreferences(scope);
            return pane;
        }
        using (var session = await Mount(window))
        {
            session.Draft!["get_settings"]!["auto_continue_limit"] = 77;
            session.KeepWindowPreferences();
        }
        using (var otherWindow = await Mount(new()))
            Check(!otherWindow.HasChanges, "preferences do not cross settings windows");
        using (var general = await Mount(window))
        {
            var form = general.Draft!["get_settings"]!;
            Check(form["auto_continue_limit"]!.GetValue<int>() == 77 && host.Writes.Count == 0,
                "navigating keeps the draft without writing settings");
            form["locale"] = "zh";
            general.KeepWindowPreferences();
        }
        using (var pet = await Mount(window))
        {
            pet.Draft!["get_settings"]!["pet_enabled"] = true;
            host.FutureFlag = false; // A different client changed an unrelated key after mount.
            host.SettingsReply = new();
            host.SettingsReply.SetException(new IOException("unavailable"));
            Check(!await pet.SaveAsync() && pet.HasChanges && window.PreviewLocale == "zh",
                "a failed save retains the combined edits");
            host.SettingsReply = null;
            Check(await pet.SaveAsync() && host.Writes.Count == 2, "one explicit retry submits one write");
            var saved = host.Writes[^1].Args["settings"]!;
            Check(saved["auto_continue_limit"]!.GetValue<int>() == 77 && saved["locale"]!.GetValue<string>() == "zh"
                && saved["pet_enabled"]!.GetValue<bool>() && !saved["future_flag"]!.GetValue<bool>(),
                "one payload includes three panes' changes and the host's most recent unrelated field");
        }
        using var back = await Mount(window);
        Check(!back.HasChanges && window.PreviewLocale == null, "successful Save clears pending edits");
        back.Draft!["get_settings"]!["locale"] = "zh";
        back.KeepWindowPreferences();
        using var cancel = await Mount(window);
        Check(cancel.CancelPreferences(), "Cancel discards shared preferences from a different pane");
        using var reopened = await Mount(window);
        Check(!reopened.HasChanges && host.Writes.Count == 2, "Cancel never writes and leaves no carried changes");
    }
    private static async Task SyncWindowDraftAsync()
    {
        var host = new Fake { EchoToken = "unexpected-fixture-echo" };
        using var window = new NativeWindowPreferences();
        async Task<NativeSettingsEditorModel> Mount()
        {
            var pane = new NativeSettingsEditorModel(host, "project-a");
            await pane.LoadAsync("get_settings");
            pane.EditWindowPreferences(window);
            return pane;
        }
        using (var sync = await Mount())
        {
            var prefs = sync.Draft!["get_settings"]!;
            Check(prefs["sync_relay_token"]!.GetValue<string>() == "" && !sync.HasChanges, "read credentials are never displayed");
            prefs["sync_folder"] = "D:/fixture/shared";
            sync.KeepWindowPreferences();
            sync.Discard(); // Opening a channel's own form drops only the mounted editor.
            sync.Edit(new() { ["app_id"] = "demo" }, "set_feishu_channel");
            sync.Draft!["app_id"] = "not-a-global-setting";
            sync.KeepWindowPreferences();
        }
        using (var session = await Mount())
        {
            session.Draft!["get_settings"]!["max_iter"] = 61;
            Check(await session.SaveAsync(), "Session can save sync settings");
            var payload = host.Writes.Single().Args["settings"]!;
            Check(payload["sync_folder"]!.GetValue<string>() == "D:/fixture/shared" && payload["max_iter"]!.GetValue<int>() == 61
                && payload["sync_relay_token"]!.GetValue<string>() == "" && payload["app_id"] == null,
                "one settings write contains both preferences, no read-back token or channel credentials");
        }
        using (var sync = await Mount())
        {
            sync.Draft!["get_settings"]!["sync_relay_token"] = "synthetic-replacement";
            sync.KeepWindowPreferences();
        }
        using var general = await Mount();
        Check(await general.SaveAsync() && host.Writes[^1].Args["settings"]!["sync_relay_token"]!.GetValue<string>() == "synthetic-replacement"
            && general.Values["get_settings"]!["sync_relay_token"]!.GetValue<string>() == "",
            "only an explicit replacement is forwarded; the acknowledged view does not keep it");
        using var reopened = await Mount();
        Check(!reopened.HasChanges && reopened.Draft!["get_settings"]!["sync_relay_token"]!.GetValue<string>() == "",
            "the window draft releases the replacement after saving");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public bool FailAppearance;
        public bool FutureFlag = true;
        public string EchoToken = "";
        public bool FailUpdate;
        public TaskCompletionSource<JsonNode?>? UpdateReply;
        public TaskCompletionSource<JsonNode?>? SettingsReply;
        public readonly List<(string Command, JsonObject Args)> Writes = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "get_settings") return Task.FromResult<JsonNode?>(new JsonObject { ["locale"] = "en", ["future_flag"] = FutureFlag, ["sync_relay_token"] = EchoToken,
                ["semantic_compact_on_model_switch"] = false, ["show_tips"] = true, ["auto_continue"] = false, ["auto_continue_limit"] = 10 });
            if (command == "get_appearance_prefs") return Task.FromResult<JsonNode?>(new JsonObject { ["theme"] = "system", ["send_with_modifier"] = false });
            if (command == "get_update_check_enabled") return Task.FromResult<JsonNode?>(JsonValue.Create(true));
            if (command == "get_bootstrap_status") return Task.FromResult<JsonNode?>(new JsonObject { ["app_version"] = "1.22.0" });
            Writes.Add((command, (JsonObject)arguments.DeepClone()));
            if (command == "set_appearance_prefs" && FailAppearance) return Task.FromException<JsonNode?>(new IOException("lost acknowledgement"));
            if (command == "set_update_check_enabled" && FailUpdate) return Task.FromException<JsonNode?>(new IOException("lost update acknowledgement"));
            if (command == "set_update_check_enabled" && UpdateReply != null) return UpdateReply.Task;
            if (command == "set_update_check_enabled") return Task.FromResult(arguments["enabled"]?.DeepClone());
            if (command == "set_settings" && SettingsReply != null) return SettingsReply.Task;
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
