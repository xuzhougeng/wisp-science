using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSkillStoreTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var model = new NativeSkillStoreModel(fake, "owned", false);
        Check(await model.LoadCatalogAsync() && model.Catalog.Count == 2 && model.InstalledNames.Contains("EXAMPLE"), "store loads directory metadata and project-scoped installed names");
        Check(fake.Calls[0].Project == null && fake.Calls[1].Project == "owned", "public directory and installed-skill scopes are distinct");
        var metadata = new JsonObject { ["required_dependencies"] = new JsonArray("Python", "R"), ["verified_wisp"] = null };
        Check(NativeSkillStoreModel.MetadataValue(metadata, "required_dependencies") == "Python; R" && NativeSkillStoreModel.MetadataValue(metadata, "verified_wisp").Contains("未验证"), "metadata preserves dependency lists and explicitly distinguishes unverified runtime behavior");
        metadata["verified_wisp"] = "1.22.0"; Check(NativeSkillStoreModel.MetadataValue(metadata, "verified_wisp") == "1.22.0", "reported verification is shown without replacing the author value");
        model.Query = "author b"; model.Tag = "research"; Check(model.Visible.Length == 0, "community text and tag filters intersect");
        model.Tag = ""; Check(model.Visible.Length == 1, "community search includes author metadata"); model.Query = "";
        model.SetMode(true); Check(model.ExactRef == "" && model.Github, "GitHub defaults leave the branch unspecified");
        model.EditSource("https://github.com/owner/repository", "release/branch");
        Check(await model.PreviewAsync() && fake.Calls[^1].Args["exactRef"]!.GetValue<string>() == "release/branch" && fake.Calls[^1].Project == "owned", "preview uses the exact user ref and owned project context");
        model.CandidateQuery = "genomics"; Check(model.VisibleCandidates.Length == 1, "package filtering includes tags");
        var selected = model.Candidates[0]; Check(model.Select(selected) && !await model.InstallAsync(), "install requires a separate source confirmation");
        Check(model.BeginConfirmation() && JsonNode.DeepEquals(model.ConfirmationSource, selected["source"]), "confirmation freezes the reviewed source");
        var calls = fake.Calls.Count; model.EditSource("https://github.com/other/repository", ""); model.SetMode(false);
        Check(model.Github && model.SourceUrl == "https://github.com/owner/repository" && fake.Calls.Count == calls, "confirmed source cannot be replaced by another tab or input edit");
        Check(model.Back() && !model.Confirming && model.Selected == selected, "first Back closes only confirmation");
        Check(model.BeginConfirmation(), "confirmation can reopen"); fake.FailInstall = true;
        Check(!await model.InstallAsync() && model.Confirming && selected["installed_source"] == null && model.Error != null, "failed installation preserves review and does not invent success");
        fake.FailInstall = false; fake.PendingInstall = new(); var installing = model.InstallAsync(); calls = fake.Calls.Count;
        Check(model.Busy && model.Back() && model.Confirming && !await model.InstallAsync() && fake.Calls.Count == calls, "pending install cannot close confirmation or submit twice");
        fake.PendingInstall.SetResult(new JsonObject { ["name"] = "package", ["directory"] = "C:/skills/package", ["notice"] = "Installed; refresh failed" }); await installing;
        Check(!model.Confirming && selected["installed_source"] != null && model.Success!.Contains("refresh failed") && !model.BeginConfirmation(), "installation acknowledgement retains its refresh notice and prevents duplicate installation");
        Check(model.Back() && model.Selected == null && model.Candidates.Count == 1, "detail returns to the candidate list");
        model.SetMode(false); Check(await model.PreviewEntryAsync(model.Catalog[0]) && model.SourceUrl.EndsWith("feature/branch/folder%20name"), "community preview preserves the full package path and exact ref");
        Check(fake.Calls[^1].Args["exactRef"]!.GetValue<string>() == "feature/branch", "a slash-containing ref is sent separately to remove ref/path ambiguity");
        Check(await model.PreviewMarketplaceAsync(2) && model.Marketplace == 2 && model.SourceUrl == NativeSkillStoreModel.Marketplaces[2].Url && model.ExactRef == "main", "marketplaces preview their explicit package directory");
        Check(model.Back() && model.Marketplace == null && model.Candidates.Count == 0, "marketplace Back returns to community browsing");
        foreach (var key in new[] { "format_errors", "resource_errors" })
        { var invalid = Fake.Candidate(); invalid[key] = new JsonArray("error"); Check(!NativeSkillStoreModel.Installable(invalid), "validation errors block installation"); }
        var conflict = Fake.Candidate(); conflict["conflict"] = ""; Check(!NativeSkillStoreModel.Installable(conflict), "even an empty conflict response is present and blocks install");
        var installed = Fake.Candidate(); installed["installed_source"] = installed["source"]!.DeepClone(); Check(!NativeSkillStoreModel.Installable(installed), "already installed packages cannot be installed again");
        var delayed = new Fake { PendingPreview = new() }; using var cancel = new NativeSkillStoreModel(delayed, "owned", true); cancel.EditSource("https://github.com/owner/repo", "");
        var pending = cancel.PreviewAsync(); var version = cancel.PreviewVersion;
        Check(cancel.Previewing && cancel.Back() && !cancel.Previewing, "Back cancels a pending read immediately");
        Check(version != cancel.PreviewVersion, "cancellation invalidates the view completion so late reads cannot rebuild focused controls");
        delayed.PendingPreview.SetResult(new JsonArray(Fake.Candidate())); await pending;
        Check(cancel.Candidates.Count == 0 && !cancel.Previewed, "cancelled late preview results cannot reappear");
        delayed.PendingPreview = new(); pending = cancel.PreviewAsync(); cancel.Dispose(); delayed.PendingPreview.SetResult(new JsonArray(Fake.Candidate())); await pending;
        Check(cancel.Candidates.Count == 0, "closed store rejects late preview results");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>
        {
            ["en"] = new() { ["store.unverified"] = "Runtime not verified", ["store.format_pass"] = "Format validation passed", ["store.invalid"] = "Validation failed", ["store.conflict"] = "Name conflict", ["store.installed"] = "Installed" },
            ["zh"] = new() { ["store.unverified"] = "运行未验证", ["store.format_pass"] = "格式校验通过" }
        });
        var localizedMetadata = new JsonObject { ["verified_wisp"] = null, ["required_dependencies"] = new JsonArray("用户工具", "R") };
        Check(NativeSkillStoreModel.MetadataValue(localizedMetadata, "verified_wisp", locale) == "Runtime not verified"
            && NativeSkillStoreModel.MetadataValue(localizedMetadata, "required_dependencies", locale) == "用户工具; R",
            "only missing verification copy follows locale; author dependency names stay verbatim");
        localizedMetadata["verified_wisp"] = "Custom author declaration";
        Check(NativeSkillStoreModel.MetadataValue(localizedMetadata, "verified_wisp", locale) == "Custom author declaration", "reported author metadata is never translated");
        var status = Fake.Candidate(); Check(NativeSkillStoreModel.CandidateStatus(status, locale) == "Format validation passed", "valid package status is localized");
        status["resource_errors"] = new JsonArray("Missing asset"); Check(NativeSkillStoreModel.CandidateStatus(status, locale) == "Validation failed", "resource failures retain their own status");
        status["installed_source"] = status["source"]!.DeepClone(); Check(NativeSkillStoreModel.CandidateStatus(status, locale) == "Installed", "installed package status takes precedence over revalidation");
        status["conflict"] = "collision"; Check(NativeSkillStoreModel.CandidateStatus(status, locale) == "Name conflict", "conflicts take precedence over installed status");
        locale.Preview("zh"); Check(NativeSkillStoreModel.CandidateStatus(Fake.Candidate(), locale) == "格式校验通过", "Chinese status uses the same keyed catalog"); locale.Preview("en");
        using var localized = new NativeSkillStoreModel(new Fake(), "owned", true, locale); localized.EditSource("https://github.com/owner/repository", "main");
        Check(await localized.PreviewAsync() && localized.Select(localized.Candidates[0]) && localized.BeginConfirmation(), "localized view retains source-review gates");
        Check(await localized.InstallAsync() && localized.Success == "Installed: C:/skills/package", "acknowledged install uses a localized prefix without changing its path");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool FailInstall;
        public TaskCompletionSource<JsonNode?>? PendingInstall, PendingPreview;
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public static JsonObject Candidate() => new() { ["name"] = "package", ["description"] = "Example", ["tags"] = new JsonArray("genomics"), ["source"] = new JsonObject { ["repository"] = "owner/repository", ["source_url"] = "https://github.com/owner/repository", ["git_ref"] = "main", ["commit"] = new string('a', 40), ["package_path"] = "skills/package" }, ["markdown"] = "# Example\nFull source", ["format_errors"] = new JsonArray(), ["resource_errors"] = new JsonArray(), ["warnings"] = new JsonArray() };
        private static JsonObject Entry(string name, string author, string tag) => new() { ["name"] = name, ["author"] = author, ["description"] = "Description", ["tags"] = new JsonArray(tag), ["repository"] = "owner/repo", ["git_ref"] = "feature/branch", ["package_path"] = "folder name" };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (command == "preview_github_skills" && PendingPreview != null) return PendingPreview.Task;
            if (command == "install_github_skill" && FailInstall) return Task.FromException<JsonNode?>(new IOException("Unconfirmed install"));
            if (command == "install_github_skill" && PendingInstall != null) return PendingInstall.Task;
            JsonNode? response = command switch
            {
                "list_community_skills" => new JsonObject { ["entries"] = new JsonArray(Entry("example", "author A", "research"), Entry("other", "author B", "writing")), ["notice"] = "Bundled directory" },
                "list_skills" => new JsonArray(new JsonObject { ["name"] = "example" }),
                "preview_github_skills" => new JsonArray(Candidate()),
                "install_github_skill" => new JsonObject { ["name"] = "package", ["directory"] = "C:/skills/package" },
                _ => throw new InvalidOperationException(command)
            };
            return Task.FromResult<JsonNode?>(response);
        }
    }
}
