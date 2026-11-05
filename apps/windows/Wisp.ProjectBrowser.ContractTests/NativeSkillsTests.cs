using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSkillsTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var scopeFake = new Fake { ScopeCatalog = true };
        using (var detail = new NativeSkillsModel(scopeFake, "owned-project"))
        {
            Check(await detail.LoadAsync() && await detail.OpenAsync("field-notes"), "project skill opens before any copy exists");
            Check(NativeSkillsModel.Copies(detail.Selected!).Promote && !await detail.ChangeScopeAsync(false), "only project copy cannot be removed");
            scopeFake.Fail = "list_skills";
            Check(!await detail.ChangeScopeAsync(true) && detail.ScopeNeedsRead && detail.PromotionReceipt != null, "a committed promotion is retained if re-reading the catalog fails");
            Check(!await detail.ChangeScopeAsync(true), "an uncertain catalog cannot replay the promotion");
            scopeFake.Fail = null;
            Check(await detail.RefreshScopeAsync() && !detail.ScopeNeedsRead && detail.Files.Contains("SKILL.md"), "read-only retry refreshes the selected package");
            Check(await detail.ChangeScopeAsync(false) && (string?)detail.Selected?["scope"] == "global" && detail.ProjectCopyRemoved, "removing a project copy keeps the surviving detail open");
            Check(scopeFake.Calls.Count(c => c.Command == "promote_skill_to_global") == 1 && scopeFake.Calls.All(c => c.Project == "owned-project"), "copy changes are pinned to one workspace and never replayed");
            var malformed = new JsonObject { ["copies"] = new JsonArray(new JsonObject { ["scope"] = "project", ["dir"] = "dir", ["effective"] = "true" }) };
            Check(!NativeSkillsModel.Copies(malformed).Promote, "incomplete legacy or malformed rows cannot authorize scope changes");
        }
        var fake = new Fake(); using var model = new NativeSkillsModel(fake, "owned-project");
        Check(await model.LoadAsync() && model.Skills.Count == 3 && fake.Calls.All(c => c.Project == "owned-project"), "skills use the page-owned project scope");
        Check(!await model.InstallAsync("C:/sample/SKILL.md", "project"), "no writes until the host identifies both destinations");
        Check(await model.ReadInstallLocationsAsync() && model.InstallLocations?.Project == "D:/study/.wisp/skills", "project destination comes from the owning host");
        Check(!await model.InstallAsync("C:/sample/SKILL.md", "other"), "unsupported scope is not sent");
        Check(await model.InstallAsync("C:/sample/SKILL.md", "project") && model.InstallReceipt?["scope"]?.GetValue<string>() == "project", "acknowledged project installs keep the receipt across reload");
        var install = fake.Calls.Single(c => c.Command == "install_scoped_skill");
        Check(install.Project == "owned-project" && (string?)install.Args["scope"] == "project" && (string?)install.Args["srcPath"] == "C:/sample/SKILL.md", "the picked source and scope travel together on the pinned project");
        fake.Fail = "skill_install_locations";
        Check(!await model.ReadInstallLocationsAsync() && !await model.InstallAsync("C:/sample", "global") && fake.Calls.Count(c => c.Command.Contains("install_scoped")) == 1,
            "older or unavailable host cannot silently fall back to a global installation");
        fake.Fail = null;
        model.Query = "SCIENCE"; model.Filter = "__enabled";
        Check(model.Visible.Length == 1, "search intersects enabled and description filters");
        model.Query = ""; model.Filter = "tag-only";
        Check(model.Visible.Length == 1, "tag chips use exact catalog tags");
        model.Query = "tag-only"; model.Filter = "";
        Check(model.Visible.Length == 0, "free-text search matches WebView names and descriptions, not hidden tags");
        Check(NativeSkillsModel.Tags(" b, a, b, , A ").SequenceEqual(new[] { "A", "a", "b" }), "tags are trimmed, unique and ordinal-sorted");
        Check(!NativeSkillsModel.CanRemove(model.Skills[0]) && NativeSkillsModel.CanRemove(model.Skills[1]) && !NativeSkillsModel.CanRemove(model.Skills[2]), "only unmanaged non-builtin global skills can be removed");
        model.Query = ""; model.Filter = ""; fake.FailName = "global-skill";
        Check(!await model.SetVisibleEnabledAsync(false) && !NativeSkillsModel.B(model.Skills[0], "enabled") && NativeSkillsModel.B(model.Skills[2], "enabled"), "bulk failure retains earlier acknowledgements and excludes managed skills");
        Check(fake.Calls.Count(c => c.Command == "set_skill_enabled") == 2, "bulk operations stop on uncertainty without replaying earlier writes"); fake.FailName = null;
        Check(await model.OpenAsync("global-skill") && model.File == "SKILL.md" && model.Content!.Contains('\n'), "detail selects SKILL.md over the first file and retains multiline content");
        var count = fake.Calls.Count; Check(!await model.LoadAsync() && !await model.ReadAsync("../secret") && fake.Calls.Count == count, "detail cannot be overwritten by list refresh or read an unlisted path");
        fake.Fail = "read_skill_file";
        Check(!await model.ReadAsync("notes.txt") && model.File == "notes.txt" && model.Content == null, "failed file switches clear the previous content"); fake.Fail = null;
        Check(await model.ReadAsync("notes.txt") && !model.IsMarkdown, "file reads can be retried explicitly");
        model.EditTags("one, two, one"); fake.Fail = "set_skill_tags";
        Check(!await model.SaveTagsAsync() && model.TagsChanged && NativeSkillsModel.Strings(model.Selected!["tags"]).Length == 0, "tag failures preserve the pending text and confirmed tags"); fake.Fail = null;
        Check(await model.SaveTagsAsync() && !model.TagsChanged && model.TagsText == "one, two", "void tag acknowledgements normalize the displayed tags");
        Check(await model.RemoveAsync() && model.Selected == null && model.Skills.Count == 2, "successful removal returns to the catalog");
        Check(await model.OpenAsync("project-skill") && !await model.RemoveAsync(), "project skill removal is blocked before invoking the host");
        model.Back(); var before = fake.Calls.Count;
        Check(!await model.SetEnabledAsync("managed-skill", false) && fake.Calls.Count == before, "managed skills cannot be toggled through the detail or list");
        var homeFake = new Fake { ActiveProject = "restored-workspace" }; using var home = new NativeSkillsModel(homeFake, null);
        Check(await home.LoadAsync() && home.ProjectId == "restored-workspace"
            && homeFake.Calls.Select(c => c.Command).SequenceEqual(new[] { "get_project_info", "list_skills" })
            && homeFake.Calls[0].Project == null && homeFake.Calls[1].Project == "restored-workspace",
            "Home skills resolve the restored host workspace instead of choosing a project by list order");
        homeFake.ActiveProject = "different-main-project";
        Check(await home.LoadAsync() && await home.OpenAsync("global-skill") && await home.SetEnabledAsync("global-skill", false)
            && homeFake.Calls.Skip(1).All(c => c.Project == "restored-workspace"),
            "reloads, detail reads and writes remain pinned even if the host main window changes projects");
        using var returned = new NativeSkillsModel(new Fake(), null, "second") { Query = "science", Filter = "__enabled" };
        Check(await returned.LoadAsync() && returned.ProjectId == "second" && returned.Visible.Length == 1, "returning from the store keeps project, search and filter context");
        var missingFake = new Fake { MissingProject = "removed" }; using var missing = new NativeSkillsModel(missingFake, null, "removed");
        Check(!await missing.LoadAsync() && missing.ProjectId == "removed" && missingFake.Calls.Count == 1,
            "an unavailable owned project reports failure instead of silently changing the configuration scope");
        var emptyFake = new Fake { ActiveProject = "" }; using var emptyHome = new NativeSkillsModel(emptyFake, null);
        Check(!await emptyHome.LoadAsync() && !emptyHome.Loaded && emptyFake.Calls.Count == 1,
            "missing host project identity never falls through to an unscoped skills command");
        var delayed = new Fake { Pending = new() }; var closing = new NativeSkillsModel(delayed, "owned"); var load = closing.LoadAsync(); closing.Dispose(); delayed.Pending.SetResult(new JsonArray()); await load;
        Check(!closing.Loaded && closing.Skills.Count == 0, "late reads cannot repopulate a closed surface");
        var readingFake = new Fake { PendingFiles = new(), PendingRead = new() }; using var reading = new NativeSkillsModel(readingFake, "owned");
        await reading.LoadAsync(); var opening = reading.OpenAsync("global-skill");
        Check(reading.Busy && reading.Selected != null && reading.Content == null && !opening.IsCompleted, "opening immediately exposes the detail loading state before the file list arrives");
        readingFake.PendingFiles.SetResult(new JsonArray("SKILL.md"));
        await readingFake.ReadStarted.Task;
        Check(reading.Busy && reading.File == "SKILL.md" && reading.Content == null, "the selected file remains in a loading state until its body arrives");
        reading.Dispose(); readingFake.PendingRead.SetResult(new JsonObject { ["path"] = "SKILL.md", ["content"] = "late body" });
        Check(!await opening && reading.Selected == null && reading.Content == null, "closing during a file read cannot restore its detail content");
        Check(NativeSkillsModel.MarkdownBody("\uFEFF---\r\nname: example\r\n---\r\n# Body") == "# Body", "Markdown preview strips a complete frontmatter block and normalizes CRLF");
        Check(NativeSkillsModel.MarkdownBody("---\nnot closed") == "---\nnot closed" && NativeSkillsModel.MarkdownBody("# First\n---\ntext") == "# First\n---\ntext", "incomplete frontmatter and later horizontal rules stay visible");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool ScopeCatalog;
        private bool hasGlobal, hasProject = true, removedGlobal;
        private JsonObject ScopedRow()
        {
            var row = Skill("field-notes", hasProject ? "project" : "global");
            var copies = new JsonArray();
            if (hasProject) copies.Add(new JsonObject { ["scope"] = "project", ["dir"] = "D:/study/notes", ["effective"] = true, ["same_as_effective"] = true });
            if (hasGlobal) copies.Add(new JsonObject { ["scope"] = "global", ["dir"] = "C:/home/notes", ["effective"] = !hasProject, ["same_as_effective"] = true });
            row["copies"] = copies; return row;
        }
        public string? Fail, FailName;
        public string ActiveProject = "first";
        public string? MissingProject;
        public TaskCompletionSource<JsonNode?>? Pending, PendingFiles, PendingRead;
        public TaskCompletionSource ReadStarted { get; } = new();
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        private static JsonObject Skill(string name, string scope, bool managed = false) => new() { ["name"] = name, ["scope"] = scope, ["enabled"] = true, ["builtin"] = false, ["managed"] = managed, ["description"] = name == "project-skill" ? "Science analysis" : "Other", ["tags"] = name == "project-skill" ? new JsonArray("tag-only") : new JsonArray() };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (Fail == command || command == "set_skill_enabled" && arguments["name"]!.GetValue<string>() == FailName) return Task.FromException<JsonNode?>(new IOException("Unconfirmed"));
            if (projectId != null && projectId == MissingProject) return Task.FromException<JsonNode?>(new IOException("Project no longer available"));
            if (command == "list_skills" && Pending != null) return Pending.Task;
            if (command == "list_skill_files" && PendingFiles != null) return PendingFiles.Task;
            if (command == "read_skill_file" && PendingRead != null) { ReadStarted.SetResult(); return PendingRead.Task; }
            if (ScopeCatalog && command == "promote_skill_to_global") hasGlobal = true;
            if (ScopeCatalog && command == "remove_project_skill_copy") hasProject = false;
            if (command == "remove_skill") removedGlobal = true;
            JsonNode? response = command switch
            {
                "get_project_info" => new JsonObject { ["id"] = ActiveProject, ["name"] = "Restored workspace" },
                "skill_install_locations" => new JsonObject { ["global_dir"] = "C:/home/.wisp/skills", ["project_dir"] = "D:/study/.wisp/skills" },
                "install_scoped_skill" => new JsonObject { ["name"] = "sample", ["scope"] = arguments["scope"]!.DeepClone(), ["dir"] = "D:/study/.wisp/skills/sample" },
                "list_skills" when ScopeCatalog => new JsonArray(ScopedRow()),
                "promote_skill_to_global" => new JsonObject { ["scope"] = "global", ["dir"] = "C:/home/notes", ["name"] = "field-notes" },
                "remove_project_skill_copy" or "get_skill_install_source" => null,
                "list_skills" when removedGlobal => new JsonArray(Skill("project-skill", "project"), Skill("managed-skill", "global", true)),
                "list_skills" => new JsonArray(Skill("project-skill", "project"), Skill("global-skill", "global"), Skill("managed-skill", "global", true)),
                "list_skill_files" => new JsonArray("notes.txt", "SKILL.md"),
                "read_skill_file" => new JsonObject { ["path"] = arguments["path"]!.DeepClone(), ["content"] = "line one\nline two" },
                "set_skill_tags" or "set_skill_enabled" or "remove_skill" => null,
                _ => throw new InvalidOperationException(command)
            };
            return Task.FromResult(response);
        }
    }
}
