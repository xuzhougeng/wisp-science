using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeNotificationSoundTests
{
    public static async Task RunAsync()
    {
        var catalog = JsonNode.Parse("""
            [{"id":"Notification.Default","label":"Notification"},
             {"id":"Notification.Mail","label":"Mail"},
             {"id":"future-sound","label":"Future sound"}]
            """);
        var options = NativeNotificationSounds.Choices(catalog, "Ping");
        Check(options[0] == ("", "系统默认") && options[2] == ("Notification.Mail", "邮件"),
            "host sound IDs retain exact values and Windows labels are localized");
        Check(options[3] == ("future-sound", "Future sound") && options[^1].Id == "Ping",
            "new host labels and a saved foreign-platform choice remain visible");
        Check(!NativeNotificationSounds.CanPreview(catalog, "Ping")
            && !NativeNotificationSounds.CanPreview(catalog, "notification.mail")
            && NativeNotificationSounds.CanPreview(catalog, "")
            && NativeNotificationSounds.CanPreview(catalog, "Notification.Mail"),
            "preview permits the platform default or an exact advertised ID only");
        Check(NativeNotificationSounds.Choices(new JsonArray(), "Ping").Length == 0
            && !NativeNotificationSounds.CanPreview(null, ""), "empty or missing catalogs never invent sounds");
        var malformed = JsonNode.Parse("""
            [null,7,{}, {"id":2}, {"id":" "}, {"id":"valid","label":9}, {"id":"valid","label":"duplicate"}]
            """);
        Check(NativeNotificationSounds.Catalog(malformed).SequenceEqual(new[] { ("valid", "valid") }),
            "malformed entries and duplicate IDs do not break the picker");

        var settings = new JsonObject { ["notifications_enabled"] = false, ["notification_sound_enabled"] = false,
            ["notification_sound"] = "Notification.Default", ["future_option"] = new JsonObject { ["keep"] = true } };
        var fake = new Fake { Catalog = catalog, Settings = settings };
        using var model = new NativeSettingsEditorModel(fake, "sound-project");
        Check(await model.LoadAsync("get_settings") && await model.LoadNotificationSoundsAsync(), "load general settings and optional catalog");
        Check(fake.Calls.Select(call => call.Command).SequenceEqual(new[] { "get_settings", "native_settings_capabilities", "list_notification_sounds" }),
            "opening settings only reads capabilities and sounds, without playing or saving");
        var count = fake.Calls.Count;
        Check(!await model.PreviewNotificationSoundAsync() && fake.Calls.Count == count, "preview requires an active settings draft");
        model.Edit(model.Values["get_settings"]!.AsObject(), "set_settings", "settings");
        model.Draft!["notification_sound"] = "Notification.Mail";
        Check(await model.PreviewNotificationSoundAsync(), "explicit preview works while the saved sound switch is off");
        var preview = fake.Calls.Last();
        Check(preview.Command == "preview_notification_sound" && preview.Project == "sound-project"
            && preview.Args.Count == 1 && preview.Args["sound"]!.GetValue<string>() == "Notification.Mail",
            "preview sends only the selected sound through the scoped host command");
        Check(model.HasChanges && !model.SaveConfirmed && !model.Draft!["notification_sound_enabled"]!.GetValue<bool>()
            && settings["notification_sound"]!.GetValue<string>() == "Notification.Default",
            "preview neither saves, enables sound, discards the draft nor mutates the original snapshot");
        model.Draft["notification_sound"] = "Ping"; count = fake.Calls.Count;
        Check(!await model.PreviewNotificationSoundAsync() && fake.Calls.Count == count, "unsupported saved sound is not sent for preview");
        model.Draft["notification_sound"] = "Notification.Mail";
        fake.PreviewFails = true; count = fake.Calls.Count;
        Check(!await model.PreviewNotificationSoundAsync() && fake.Calls.Count == count + 1
            && model.HasChanges && model.Error != null, "failed preview retains the draft and is never replayed");
        fake.PreviewFails = false;
        model.Draft["notification_sound_enabled"] = true;
        Check(await model.SaveAsync(), "sound changes save only on explicit save");
        var saved = fake.Calls.Last().Args["settings"]!;
        Check(fake.Calls.Last().Command == "set_settings" && saved["notification_sound_enabled"]!.GetValue<bool>()
            && !saved["notifications_enabled"]!.GetValue<bool>() && saved["future_option"]!["keep"]!.GetValue<bool>(),
            "sound can be enabled independently of banners while preserving unknown settings");

        var oldHost = new Fake { Settings = settings, Capabilities = new JsonObject { ["commands"] = new JsonArray("get_settings") } };
        using var legacy = new NativeSettingsEditorModel(oldHost, null);
        await legacy.LoadAsync("get_settings"); await legacy.LoadNotificationSoundsAsync();
        Check(oldHost.Calls.Count == 2 && legacy.Values["get_settings"] != null && legacy.Error == null
            && legacy.Values["list_notification_sounds"] == null, "old host keeps general settings usable without requesting unsupported commands");
        oldHost.Capabilities = new JsonObject { ["commands"] = new JsonArray("list_notification_sounds") };
        Check(!NativeNotificationSounds.Supported(oldHost.Capabilities), "catalog alone does not advertise preview support");

        var failedHost = new Fake { Settings = settings, CatalogFails = true };
        using var failed = new NativeSettingsEditorModel(failedHost, null);
        await failed.LoadAsync("get_settings");
        Check(!await failed.LoadNotificationSoundsAsync() && failed.Values["get_settings"] != null
            && failed.Error != null, "catalog failure is visible without discarding successfully loaded general settings");

        var delayedHost = new Fake { PendingCapabilities = new() };
        using var closed = new NativeSettingsEditorModel(delayedHost, null);
        var loading = closed.LoadNotificationSoundsAsync();
        closed.Dispose(); delayedHost.PendingCapabilities.SetResult(delayedHost.Capabilities);
        Check(!await loading && delayedHost.Calls.Count == 1 && closed.Values.Count == 0,
            "closing during capabilities prevents a later catalog call or stale state");
    }

    private static void Check(bool value, string description)
    { if (!value) throw new InvalidOperationException(description); Console.WriteLine("PASS notification sounds: " + description); }

    private sealed class Fake : INativeSettingsClient
    {
        public JsonNode? Catalog, Settings;
        public JsonObject Capabilities = new() { ["commands"] = new JsonArray("list_notification_sounds", "preview_notification_sound") };
        public bool PreviewFails, CatalogFails;
        public TaskCompletionSource<JsonNode?>? PendingCapabilities;
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            return command switch
            {
                "get_settings" => Task.FromResult(Settings),
                "native_settings_capabilities" => PendingCapabilities?.Task ?? Task.FromResult<JsonNode?>(Capabilities),
                "list_notification_sounds" when CatalogFails => Task.FromException<JsonNode?>(new IOException("catalog unavailable")),
                "list_notification_sounds" => Task.FromResult(Catalog),
                "preview_notification_sound" when PreviewFails => Task.FromException<JsonNode?>(new IOException("playback unavailable")),
                "preview_notification_sound" or "set_settings" => Task.FromResult<JsonNode?>(null),
                _ => throw new InvalidOperationException("Unexpected command: " + command)
            };
        }
    }
}
