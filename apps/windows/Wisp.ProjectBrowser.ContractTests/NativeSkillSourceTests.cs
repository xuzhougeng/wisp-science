using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSkillSourceTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var client = new Fake(); using var model = new NativeSkillsModel(client, "owned-project");
        await model.LoadAsync(); await model.OpenAsync("first");
        model.SetSourceMode(true); await model.ReadAsync("notes.txt"); await model.ReadAsync("guide.md");
        Check(model.SourceMode && model.IsMarkdown, "source mode survives file changes, including a non-Markdown file");
        client.FailRead = true; await model.ReadAsync("SKILL.md");
        Check(model.SourceMode && model.Content == null, "failed file switching retains source preference without stale content");
        client.FailRead = false; await model.ReadAsync("SKILL.md");
        model.SetSourceMode(false); Check(!model.SourceMode, "the Preview action returns to rendered Markdown");
        await model.LoadInstallSourceAsync();
        Check(model.InstallSource?["commit"]?.GetValue<string>() == "abcdef" && model.Content == "# SKILL.md" && model.Error == null,
            "optional installation source coexists with the selected file");
        Check(client.Calls.All(c => c.Project == "owned-project")
            && client.Calls.Single(c => c.Command == "get_skill_install_source").Args["name"]?.GetValue<string>() == "first",
            "source lookup uses the selected name and owned project scope");
        model.SetSourceMode(true); model.Back(); await model.OpenAsync("second");
        Check(!model.SourceMode && model.InstallSource == null, "a new skill starts in Preview without the previous origin");
        client.Source = null; await model.LoadInstallSourceAsync();
        Check(model.InstallSource == null && model.SourceError == null, "bundled or local packages with no origin show no fabricated source");
        client.Source = new JsonObject { ["commit"] = 7 }; await model.LoadInstallSourceAsync();
        Check(model.SourceError != null && model.InstallSource == null && model.Content != null && model.Error == null,
            "malformed optional provenance leaves document browsing available");
        client.Source = Source(); await model.LoadInstallSourceAsync();
        Check(model.SourceError == null && model.InstallSource != null, "explicit retry restores valid provenance");

        client.Pending = new(); var pending = model.LoadInstallSourceAsync();
        Check(model.SourceLoading && !model.Busy && await model.ReadAsync("guide.md"), "origin lookup does not lock file browsing");
        model.Back(); await model.OpenAsync("first"); var oldPending = client.Pending; client.Pending = null;
        await model.LoadInstallSourceAsync(); oldPending.SetResult(new JsonObject { ["commit"] = "stale and malformed" }); await pending;
        Check(model.Selected?["name"]?.GetValue<string>() == "first" && model.InstallSource?["commit"]?.GetValue<string>() == "abcdef"
            && !model.SourceLoading && model.SourceError == null, "late origin success or validation failure cannot contaminate another skill");
        client.Pending = new(); var closing = model.LoadInstallSourceAsync(); model.Dispose();
        client.Pending.SetException(new IOException("late failure")); await closing;
        Check(model.InstallSource == null && model.SourceError == null && !model.SourceLoading, "closing invalidates pending source replies and errors");
        Check(client.Calls.All(c => c.Command is "list_skills" or "list_skill_files" or "read_skill_file" or "get_skill_install_source"),
            "browsing source information never installs or mutates a skill");

        var source = Source(); source["future_field"] = "retained";
        var copy = NativeSkillSourcePresentation.Parse(source)!; source["commit"] = "changed";
        Check(copy["commit"]?.GetValue<string>() == "abcdef" && copy["future_field"]?.GetValue<string>() == "retained", "origin projection owns its copy and tolerates future fields");
        var entries = NativeSkillSourcePresentation.Entries(copy);
        Check(entries.Select(e => e.Label).SequenceEqual(new[] { "GitHub", "Source URL", "SKILL.md", "Ref", "Commit", "Package" })
            && entries[2].Url == "https://github.com/example/research/blob/abcdef/skills/path%20with%20spaces/%E7%A0%94%E7%A9%B6/SKILL.md"
            && entries[4].Code && entries[4].Value == "abcdef", "source rows match WebView and link to the immutable commit with escaped paths");
        copy["source_url"] = "javascript:alert(1)"; copy["package_path"] = "";
        entries = NativeSkillSourcePresentation.Entries(copy);
        Check(entries[1].Url == null && entries[5].Value == "/" && entries[2].Url!.EndsWith("/abcdef/SKILL.md"), "unsafe source links stay plain text and root packages have the correct file URL");
    }
    private static JsonObject Source() => new() { ["repository"] = "example/research", ["source_url"] = "https://github.com/example/research/tree/main/skills/path%20with%20spaces",
        ["git_ref"] = "main", ["commit"] = "abcdef", ["package_path"] = "skills/path with spaces/研究" };
    private sealed class Fake : INativeSettingsClient
    {
        public JsonNode? Source = NativeSkillSourceTests.Source();
        public bool FailRead;
        public TaskCompletionSource<JsonNode?>? Pending;
        public List<(string Command, JsonObject Args, string? Project)> Calls = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (command == "get_skill_install_source" && Pending != null) return Pending.Task;
            if (command == "read_skill_file" && FailRead) return Task.FromException<JsonNode?>(new IOException("Read failed"));
            return Task.FromResult<JsonNode?>(command switch {
                "list_skills" => new JsonArray(new JsonObject { ["name"] = "first" }, new JsonObject { ["name"] = "second" }),
                "list_skill_files" => new JsonArray("SKILL.md", "notes.txt", "guide.md"),
                "read_skill_file" => new JsonObject { ["path"] = arguments["path"]!.DeepClone(), ["content"] = "# " + arguments["path"]!.GetValue<string>() },
                "get_skill_install_source" => Source?.DeepClone(), _ => throw new InvalidOperationException(command) });
        }
    }
}
