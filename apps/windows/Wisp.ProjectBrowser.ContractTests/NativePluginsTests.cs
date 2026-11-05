using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativePluginsTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var model = new NativePluginsModel(fake, null);
        Check(await model.LoadAsync() && model.ProjectId == "q" && model.EnabledCount == 1 && fake.Calls[^1].Project == "q"
            && fake.Calls[0].Command == "get_project_info", "home plugin settings resolve the restored workspace rather than the first catalog project");
        model.Query = "  SECOND  "; Check(model.Visible.Length == 1, "plugin search matches description without changing enabled totals"); model.Query = "sample.id"; Check(model.Visible.Length == 2, "search also matches exact plugin id"); model.Query = "";
        fake.ActiveProject = "another-workspace";
        Check(await model.LoadAsync() && fake.Calls[^1].Project == "q" && fake.Calls.Count(c => c.Command == "get_project_info") == 1,
            "reload pins the page's restored workspace even if the host's active workspace changes");
        var old = model.Plugins[0]; var newer = model.Plugins[1]; fake.Fail = "set_plugin_enabled";
        Check(!await model.SetEnabledAsync(newer, true) && NativeSkillsModel.B(old, "enabled") && !NativeSkillsModel.B(newer, "enabled"), "failed binding leaves both versions unchanged");
        var calls = fake.Calls.Count; Check(!await model.UseAsync(newer) && fake.Calls.Count == calls + 1, "failed enable never creates or sends a conversation"); fake.Fail = null;
        Check(await model.SetEnabledAsync(newer, true) && !NativeSkillsModel.B(old, "enabled") && NativeSkillsModel.B(newer, "enabled") && fake.Calls[^1].Args["version"]!.GetValue<string>() == "2", "binding acknowledgement replaces only the same plugin id's prior enabled version");
        Check(model.OpenInstall() && !model.Remote && !model.CanInstall, "install starts as an empty local ZIP modal");
        model.EditInstall(path: "C:/tmp/plugin.zip", checksum: "bad"); Check(!model.CanInstall && !await model.InstallAsync(), "invalid optional checksum blocks local installation");
        model.EditInstall(checksum: ""); fake.Fail = "install_plugin"; Check(model.CanInstall && !await model.InstallAsync() && model.Installing && model.LocalPath.EndsWith(".zip"), "failed install retains the source and modal"); fake.Fail = null;
        Check(await model.InstallAsync() && !model.Installing && model.Notice != null && fake.Calls[^1].Args["expectedSha256"] == null && fake.Calls[^1].Args["srcPath"]!.GetValue<string>() == "C:/tmp/plugin.zip", "local install uses the shared camel-case contract and optional null checksum");
        Check(model.OpenInstall(), "installer can reopen"); model.SetRemote(true); model.EditInstall(url: " https://example.org/plugin.zip ", checksum: new string('A', 64));
        Check(model.CanInstall && NativePluginsModel.ValidChecksum(" " + new string('b', 64) + " ") && !NativePluginsModel.ValidChecksum(new string('z', 64)), "remote checksum validation accepts only 64 hexadecimal characters");
        Check(await model.InstallAsync() && fake.Calls[^1].Command == "install_plugin_url" && fake.Calls[^1].Args["sourceUrl"]!.GetValue<string>() == "https://example.org/plugin.zip", "remote install trims source and checksum");
        Check(model.OpenInstall() && model.CloseModal() && !model.ModalOpen && model.Plugins.Count > 0, "Escape-equivalent closure preserves the plugin parent list");
        calls = fake.Calls.Count;
        Check(model.OpenInstall(), "fresh modal opens for source-tab editing");
        model.EditInstall(path: "C:/tmp/preview-only.zip", checksum: new string('c', 64));
        model.SetRemote(true); model.EditInstall(url: "https://example.invalid/preview-only.zip"); model.SetRemote(false);
        Check(model.LocalPath == "C:/tmp/preview-only.zip" && model.SourceUrl == "https://example.invalid/preview-only.zip"
            && model.Checksum == new string('c', 64) && model.CanInstall && fake.Calls.Count == calls,
            "source tabs preserve unsaved fields without fetching or installing anything");
        Check(model.CloseModal() && model.OpenInstall() && !model.Remote && model.LocalPath == "" && model.SourceUrl == ""
            && model.Checksum == "" && !model.CanInstall && fake.Calls.Count == calls,
            "dismissal discards draft fields; reopening starts empty without touching installed plugins");
        model.CloseModal();
        var installed = model.Plugins.Single(row => NativeSkillsModel.S(row, "id") == "installed");
        Check(!await model.RemoveAsync() && model.RequestRemoval(installed), "removal needs the reviewed plugin identity"); fake.Fail = "remove_plugin";
        Check(!await model.RemoveAsync() && model.Removal == installed && model.Plugins.Contains(installed), "failed removal preserves confirmation and row"); fake.Fail = null;
        Check(await model.RemoveAsync() && model.Removal == null && !model.Plugins.Contains(installed), "acknowledged removal changes only that exact installed version");
        newer = model.Plugins.Single(row => NativeSkillsModel.S(row, "version") == "2");
        newer["enabled"] = false; fake.Fail = "native_conversation_create";
        Check(!await model.UseAsync(newer) && NativeSkillsModel.B(newer, "enabled") && model.Launch == null, "session-creation failure retains an acknowledged binding without inventing a session"); fake.Fail = null;
        fake.PendingCreate = new(); var starting = model.UseAsync(newer); calls = fake.Calls.Count;
        Check(model.Busy && !model.OpenInstall() && !await model.UseAsync(newer) && fake.Calls.Count == calls, "pending plugin startup cannot create duplicate sessions");
        fake.PendingCreate.SetResult(JsonValue.Create("new-session")); Check(await starting && model.Launch?.ProjectId == "q" && model.Launch.SessionId == "new-session" && model.Launch.SkillNames.SequenceEqual(new[] { "plugin-guide" }) && model.Launch.Prompt.Contains("技能说明"), "plugin startup retains owned session and explicit skill guidance");
        fake.PendingCreate = null; newer["runtime_status"] = "unavailable"; calls = fake.Calls.Count; Check(!await model.UseAsync(newer) && fake.Calls.Count == calls, "unavailable runtime blocks startup");
        var stale = (JsonObject)newer.DeepClone(); Check(!await model.SetEnabledAsync(stale, true) && !model.RequestRemoval(stale), "detached plugin objects cannot mutate the catalog");
        var delayed = new Fake { PendingList = new() }; var disposed = new NativePluginsModel(delayed, "owned"); var load = disposed.LoadAsync(); disposed.Dispose(); delayed.PendingList.SetResult(new JsonArray(Fake.Row("1", false))); await load;
        Check(disposed.Plugins.Count == 0, "closed page rejects late catalog reads");
        foreach (var identity in new JsonNode?[] { null, JsonValue.Create(""), JsonValue.Create("  "), JsonValue.Create(42) })
        {
            var missing = new Fake { ActiveProject = identity }; using var unavailable = new NativePluginsModel(missing, null);
            Check(!await unavailable.LoadAsync() && !unavailable.Loaded && unavailable.ProjectId == null && !unavailable.OpenInstall()
                && missing.Calls.Count == 1 && missing.Calls[0].Command == "get_project_info",
                "invalid restored identity never falls back to an unrelated project or allows installation");
        }
        var ownedFake = new Fake(); using var owned = new NativePluginsModel(ownedFake, "owned");
        Check(await owned.LoadAsync() && ownedFake.Calls.All(c => c.Command == "list_plugins" && c.Project == "owned"),
            "a project-owned page does not resolve or replace its caller's scope");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>
        {
            ["en"] = new() { ["plugins.runtime_ready"] = "Runtime ready", ["plugins.runtime_unavailable"] = "Runtime unavailable", ["plugins.runtime_not_applicable"] = "No runtime required", ["plugins.installed"] = "Installed notice" },
            ["zh"] = new() { ["plugins.runtime_ready"] = "运行环境就绪" }
        });
        var localizedRow = Fake.Row("1", false);
        Check(NativePluginsModel.RuntimeLabel(localizedRow, locale) == "Runtime ready", "ready status uses the interface locale");
        localizedRow["runtime_status"] = "unavailable";
        Check(NativePluginsModel.RuntimeLabel(localizedRow, locale) == "Runtime unavailable", "unavailable status keeps its distinct translated label");
        localizedRow["runtime_status"] = "none";
        Check(NativePluginsModel.RuntimeLabel(localizedRow, locale) == "No runtime required", "plugins without a runtime use the shared fallback label");
        localizedRow["runtime_status"] = "ready"; locale.Preview("zh");
        Check(NativePluginsModel.RuntimeLabel(localizedRow, locale) == "运行环境就绪" && localizedRow["display_name"]!.GetValue<string>() == "Example",
            "changing locale translates status without changing plugin-owned names");
        locale.Preview("en"); using var localized = new NativePluginsModel(new Fake(), "p", locale);
        Check(await localized.LoadAsync() && localized.OpenInstall(), "localized installer loads the same project contract");
        localized.EditInstall(path: "C:/tmp/preview-only.zip");
        Check(await localized.InstallAsync() && localized.Notice == "Installed notice", "successful installation notice comes from the shared locale catalog");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public string? Fail;
        public JsonNode? ActiveProject = JsonValue.Create("q");
        public TaskCompletionSource<JsonNode?>? PendingCreate, PendingList;
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public static JsonObject Row(string version, bool enabled) => new() { ["id"] = "sample.id", ["version"] = version, ["display_name"] = "Example", ["description"] = version == "2" ? "second version" : "first version", ["enabled"] = enabled, ["runtime_status"] = "ready", ["skill_names"] = new JsonArray("plugin-guide") };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (Fail == command) return Task.FromException<JsonNode?>(new IOException("Unknown result"));
            if (command == "list_plugins" && PendingList != null) return PendingList.Task;
            if (command == "native_conversation_create" && PendingCreate != null) return PendingCreate.Task;
            JsonNode? result = command switch
            {
                "get_project_info" => new JsonObject { ["id"] = ActiveProject?.DeepClone() },
                "list_plugins" => new JsonArray(Row("1", true), Row("2", false)),
                "install_plugin" or "install_plugin_url" => new JsonObject { ["id"] = "installed", ["version"] = "1", ["display_name"] = "Installed", ["enabled"] = false },
                "set_plugin_enabled" or "remove_plugin" => null,
                "native_conversation_create" => JsonValue.Create("created"),
                _ => throw new InvalidOperationException(command)
            };
            return Task.FromResult(result);
        }
    }
}
