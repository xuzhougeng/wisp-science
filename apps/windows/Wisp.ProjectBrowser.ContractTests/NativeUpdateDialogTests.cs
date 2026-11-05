using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeUpdateDialogTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        Check(NativeUpdateNotes.ForDisplay("<!-- release-title: version -->\n# Notes\nA <!-- hidden --> fix") == "\n# Notes\nA  fix",
            "release metadata and inline HTML comments are hidden like the WebView");
        Check(NativeUpdateNotes.ForDisplay("Title\r\n\r\nKeep `<!-- literal -->` <!-- hide -->here") ==
            "Title\r\n\r\nKeep `<!-- literal -->` here", "inline offsets preserve CRLF blocks and neighboring code spans");
        var codeComments = "```html\n<!-- example -->\n```\n\n`<!-- inline code -->`\n\n    <!-- indented code -->\n\n<div>inert HTML</div>";
        Check(NativeUpdateNotes.ForDisplay(codeComments) == codeComments, "code examples and non-comment HTML are preserved");
        Check(NativeUpdateNotes.ForDisplay("<!-- unfinished\nmetadata") == "", "unfinished HTML comment stays hidden");
        var actions = NativeActionLayout.Arrange([new(140, 38), new(70, 38), new(130, 38)], 436, gap: 10, rightAligned: true, firstItemLeading: true);
        Check(actions[0].X == 0 && actions[2].X + actions[2].Width == 436 && actions[2].X - actions[1].X - actions[1].Width == 10,
            "reminder action aligns left while Later/Releases retain their right alignment and gap");
        var wrapped = NativeActionLayout.Arrange([new(200, 48), new(100, 48), new(190, 48)], 380, gap: 10, rightAligned: true, firstItemLeading: true);
        Check(wrapped[0].X == 0 && wrapped[2].Y > wrapped[1].Y && wrapped.All(s => s.X >= 0 && s.X + s.Width <= 380), "large labels wrap without leaving the modal");
        var client = new Fake();
        using var modal = new NativeUpdateCheckModel(client);
        Check(modal.TitleKey == "update_modal.checking_title" && !await modal.DisableRemindersAsync(), "checking has no writable action");
        Check(await modal.LoadAsync() && modal.Available && modal.Version == "2.0.0" && modal.Notes.Contains("**fix**"), "available retains Markdown and version");
        Check(client.Commands.SequenceEqual(new[] { "check_for_updates" }), "opening is read-only");
        Check(await modal.DisableRemindersAsync() && modal.RemindersDisabled && !await modal.DisableRemindersAsync()
            && client.Commands.Count == 2, "one explicit opt-out and no replay");
        client.Status["update_available"] = false;
        using var current = new NativeUpdateCheckModel(client);
        Check(await current.LoadAsync() && current.Version == "1.20.0" && current.TitleKey == "update_modal.up_to_date_title"
            && !await current.DisableRemindersAsync(), "up-to-date uses installed version and OK only");
        client.Status["update_available"] = true;
        foreach (var url in new[] { "javascript:alert(1)", "file:///private", "http://example.com", "https://user:password@example.com" })
        {
            client.Status["release_url"] = url;
            using var invalid = new NativeUpdateCheckModel(client); await invalid.LoadAsync();
            Check(invalid.ReleaseUrl == NativeUpdateCheckModel.Releases, "invalid release target falls back to the shared releases page");
        }
        client.FailRead = true;
        using var failed = new NativeUpdateCheckModel(client);
        Check(!await failed.LoadAsync() && failed.TitleKey == "update_modal.failed_title" && failed.ReleaseUrl == NativeUpdateCheckModel.Releases,
            "failed check retains a usable Releases fallback");
        client.FailRead = false; client.BadAcknowledgement = true;
        using var uncertain = new NativeUpdateCheckModel(client); await uncertain.LoadAsync();
        var before = client.Commands.Count;
        Check(!await uncertain.DisableRemindersAsync() && uncertain.Error != null && !uncertain.RemindersDisabled
            && !await uncertain.DisableRemindersAsync() && client.Commands.Count == before + 1, "ambiguous opt-out is not confirmed or replayed");
        client.BadAcknowledgement = false;
        client.Pending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var closing = new NativeUpdateCheckModel(client); var pending = closing.LoadAsync();
        before = client.Commands.Count;
        Check(!await closing.LoadAsync() && client.Commands.Count == before, "duplicate reads cannot race");
        closing.Dispose(); client.Pending.SetResult(client.Status.DeepClone());
        Check(!await pending && closing.Result == null, "late result cannot revive a closed modal");
        client.Pending = null;

        using var editor = new NativeSettingsEditorModel(client, "project-a");
        await editor.LoadAsync("get_settings", "get_update_check_enabled");
        editor.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        editor.Draft!["get_settings"]!["locale"] = "zh";
        using var settingsModal = new NativeUpdateCheckModel(client, () => editor.SetUpdateCheckEnabledAsync(false));
        await settingsModal.LoadAsync(); before = client.Commands.Count;
        Check(await settingsModal.DisableRemindersAsync() && editor.HasChanges && editor.Draft["get_settings"]!["locale"]!.GetValue<string>() == "zh",
            "settings update dialog preserves unrelated unsaved draft");
        editor.Discard();
        Check(!editor.Values["get_update_check_enabled"]!.GetValue<bool>() && client.Commands.Count == before + 1,
            "General Cancel cannot undo or replay independently committed reminder preference");
        using var uncertainEditor = new NativeSettingsEditorModel(client, "project-a");
        await uncertainEditor.LoadAsync("get_update_check_enabled"); client.BadAcknowledgement = true;
        using var uncertainSettingsModal = new NativeUpdateCheckModel(client, () => uncertainEditor.SetUpdateCheckEnabledAsync(false));
        await uncertainSettingsModal.LoadAsync(); before = client.Commands.Count;
        Check(!await uncertainSettingsModal.DisableRemindersAsync() && uncertainEditor.Values["get_update_check_enabled"]!.GetValue<bool>()
            && !uncertainSettingsModal.RemindersDisabled && !await uncertainSettingsModal.DisableRemindersAsync() && client.Commands.Count == before + 1,
            "General's update dialog also rejects a mismatched acknowledgement without replaying the write");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public readonly List<string> Commands = [];
        public bool FailRead, BadAcknowledgement;
        public TaskCompletionSource<JsonNode?>? Pending;
        public JsonObject Status = new() { ["current_version"] = "1.20.0", ["latest_version"] = "2.0.0", ["update_available"] = true,
            ["notes"] = "## Notes\n**fix**", ["release_url"] = NativeUpdateCheckModel.Releases, ["install_supported"] = false };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Commands.Add(command);
            return command switch
            {
                "check_for_updates" => Pending?.Task ?? (FailRead ? Task.FromException<JsonNode?>(new IOException("offline")) : Task.FromResult<JsonNode?>(Status.DeepClone())),
                "set_update_check_enabled" when args["enabled"]?.GetValue<bool>() == false => Task.FromResult<JsonNode?>(JsonValue.Create(BadAcknowledgement)),
                "get_update_check_enabled" => Task.FromResult<JsonNode?>(JsonValue.Create(true)),
                "get_settings" => Task.FromResult<JsonNode?>(new JsonObject { ["locale"] = "en" }),
                _ => throw new InvalidOperationException("Unexpected command: " + command)
            };
        }
    }
}
