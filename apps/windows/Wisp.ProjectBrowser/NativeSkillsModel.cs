using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Project-scoped skill browsing. Only acknowledged writes change the displayed catalog.
public sealed class NativeSkillsModel(INativeSettingsClient client, string? initialProject, string? preferredProject = null) : WorkspaceActionModel
{
    public IReadOnlyList<JsonObject> Skills { get; private set; } = [];
    public string? ProjectId { get; private set; } = initialProject ?? preferredProject;
    public bool Loaded { get; private set; }
    public (string Global, string Project)? InstallLocations { get; private set; }
    public JsonObject? InstallReceipt { get; private set; }
    private int locationRequest;
    public async Task<bool> ReadInstallLocationsAsync()
    {
        var project = ProjectId; var ticket = ++locationRequest;
        InstallLocations = null;
        if (Closed || !Loaded || project == null) return false;
        try
        {
            var paths = await client.InvokeAsync("skill_install_locations", new(), project);
            var global = S(paths, "global_dir"); var local = S(paths, "project_dir");
            if (Closed || ticket != locationRequest || ProjectId != project ||
                string.IsNullOrWhiteSpace(global) || string.IsNullOrWhiteSpace(local)) return false;
            InstallLocations = (global, local); return true;
        }
        catch { return false; }
    }
    public string Query { get; set; } = "";
    public string Filter { get; set; } = "";
    public JsonObject? Selected { get; private set; }
    public IReadOnlyList<string> Files { get; private set; } = [];
    public string? File { get; private set; }
    public string? Content { get; private set; }
    public bool SourceMode { get; private set; }
    public JsonObject? InstallSource { get; private set; }
    public bool SourceLoading { get; private set; }
    public string? SourceError { get; private set; }
    private int sourceGeneration;
    public string TagsText { get; private set; } = "";
    public bool TagsChanged => Selected != null && !Tags(TagsText).SequenceEqual(Strings(Selected["tags"]));
    public bool IsMarkdown => File?.EndsWith(".md", StringComparison.OrdinalIgnoreCase) == true;
    public static string S(JsonNode? value, string key) => value?[key]?.GetValue<string>() ?? "";
    public static bool B(JsonNode? value, string key) => value?[key]?.GetValue<bool>() == true;
    public static string[] Strings(JsonNode? value) => (value as JsonArray)?.Select(n => n!.GetValue<string>()).ToArray() ?? [];
    private static JsonObject[] Rows(JsonNode? value) => value is JsonArray array ? array.OfType<JsonObject>().Select(row => (JsonObject)row.DeepClone()).ToArray() : throw new InvalidDataException("技能列表响应无效。");
    public static string[] Tags(string raw) => raw.Split(',').Select(t => t.Trim()).Where(t => t.Length > 0).Distinct(StringComparer.Ordinal).Order(StringComparer.Ordinal).ToArray();
    public static bool CanRemove(JsonNode row) => S(row, "scope") == "global" && !B(row, "builtin") && !B(row, "managed");
    public sealed record Copy(string Scope, string Directory, bool Effective, bool SameAsEffective);
    public sealed record CopySet(Copy[] Items, bool Promote, bool RemoveProject, bool Duplicate);
    public static CopySet Copies(JsonObject row)
    {
        CopySet unknown = new([], false, false, false);
        if (row["copies"] is not JsonArray array) return unknown; // Older hosts have no proven fallback.
        var items = new List<Copy>();
        foreach (var entry in array)
        {
            if (entry is not JsonObject copy ||
                copy["scope"] is not JsonValue sk || !sk.TryGetValue<string>(out var scope) ||
                scope is not ("bundled" or "project" or "global" or "extra" or "plugin" or "custom") ||
                copy["dir"] is not JsonValue dk || !dk.TryGetValue<string>(out var dir) || string.IsNullOrWhiteSpace(dir) ||
                copy["effective"] is not JsonValue ek || !ek.TryGetValue<bool>(out var effective) ||
                copy["same_as_effective"] is not JsonValue hk || !hk.TryGetValue<bool>(out var same)) return unknown;
            items.Add(new(scope, dir, effective, same));
        }
        if (items.Count(c => c.Effective) != 1) return unknown;
        var projects = items.Where(c => c.Scope == "project").ToArray();
        var global = items.Any(c => c.Scope == "global");
        var project = projects.Length == 1 ? projects[0] : null;
        var duplicate = project?.SameAsEffective == true && items.Any(c => c.Scope != "project" && c.SameAsEffective);
        return new(items.ToArray(), project != null && !global, project != null && (global || duplicate), duplicate && !global);
    }
    private readonly HashSet<string> scopeNeedsRead = new(StringComparer.Ordinal);
    public bool ScopeNeedsRead => Selected != null && scopeNeedsRead.Contains(S(Selected, "name"));
    public JsonObject? PromotionReceipt { get; private set; }
    public bool ProjectCopyRemoved { get; private set; }
    public async Task<bool> ChangeScopeAsync(bool promote, Func<Exception, string>? explain = null)
    {
        if (Closed || Busy || ProjectId == null || Selected == null || TagsChanged || ScopeNeedsRead) return false;
        var copies = Copies(Selected);
        if (!(promote ? copies.Promote : copies.RemoveProject)) return false;
        var name = S(Selected, "name"); scopeNeedsRead.Add(name); PromotionReceipt = null; ProjectCopyRemoved = false;
        if (!await RunAsync(async () =>
        {
            var result = await client.InvokeAsync(promote ? "promote_skill_to_global" : "remove_project_skill_copy", new() { ["name"] = name }, ProjectId);
            if (promote && (result is not JsonObject receipt || S(receipt, "name") != name || S(receipt, "scope") != "global" || string.IsNullOrWhiteSpace(S(receipt, "dir"))))
                throw new InvalidDataException("Invalid skill promotion receipt.");
            return result as JsonObject;
        }, receipt => { PromotionReceipt = receipt; ProjectCopyRemoved = !promote; }, explain)) return false;
        return await RefreshScopeAsync();
    }
    public async Task<bool> RefreshScopeAsync()
    {
        if (Closed || Busy || Selected == null || TagsChanged) return false;
        var name = S(Selected, "name");
        if (!await RunAsync(async () => Rows(await client.InvokeAsync("list_skills", new(), ProjectId)), rows =>
        {
            var receipt = PromotionReceipt; var removed = ProjectCopyRemoved;
            ClearSelection(); Skills = rows; scopeNeedsRead.Remove(name);
            Selected = rows.FirstOrDefault(row => S(row, "name") == name);
            if (Selected != null) TagsText = string.Join(", ", Strings(Selected["tags"]));
            PromotionReceipt = receipt; ProjectCopyRemoved = removed;
        }, ex => ex.Message)) return false;
        if (Selected == null) return true;
        var files = await LoadFilesAsync();
        if (!Closed && S(Selected, "name") == name) await LoadInstallSourceAsync();
        return files;
    }
    public static bool Matches(JsonObject row, string tag, string query)
    {
        var matchesTag = tag switch { "" => true, "__untagged" => Strings(row["tags"]).Length == 0, "__enabled" => B(row, "enabled"), "__disabled" => !B(row, "enabled"), _ => Strings(row["tags"]).Contains(tag, StringComparer.Ordinal) };
        var q = query.Trim();
        return matchesTag && (S(row, "name").Contains(q, StringComparison.OrdinalIgnoreCase) || S(row, "description").Contains(q, StringComparison.OrdinalIgnoreCase));
    }
    public JsonObject[] Visible => Skills.Where(row => Matches(row, Filter, Query)).ToArray();
    public string[] AllTags => Skills.SelectMany(row => Strings(row["tags"])).Distinct(StringComparer.Ordinal).Order(StringComparer.Ordinal).ToArray();
    public Task<bool> LoadAsync(bool rescan = false)
    {
        if (Selected != null) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            // Home follows the host's restored workspace, just like WebView.
            // Pin the resolved id so later reads/writes cannot drift with main.
            var project = await NativeSettingsWorkspace.ResolveAsync(client, ProjectId);
            var skills = Rows(await client.InvokeAsync(rescan ? "reload_skills" : "list_skills", new(), project));
            return (project, skills);
        }, result => { ProjectId = result.project; Skills = result.skills; scopeNeedsRead.Clear(); Loaded = true; });
    }
    public async Task<bool> OpenAsync(string name)
    {
        if (Busy || Closed || TagsChanged || !Skills.Any(s => S(s, "name") == name)) return false;
        ClearSelection();
        Selected = Skills.First(s => S(s, "name") == name); TagsText = string.Join(", ", Strings(Selected["tags"]));
        return await LoadFilesAsync();
    }
    public void SetSourceMode(bool source) { if (!Closed && Selected != null) SourceMode = source; }
    // Optional provenance must not block file browsing or apply to a later detail.
    public async Task LoadInstallSourceAsync()
    {
        if (Closed || Selected == null || SourceLoading) return;
        var name = S(Selected, "name"); var project = ProjectId; var generation = ++sourceGeneration;
        SourceLoading = true; SourceError = null; Notify();
        try
        {
            var response = await client.InvokeAsync("get_skill_install_source", new() { ["name"] = name }, project);
            var source = NativeSkillSourcePresentation.Parse(response);
            if (!Closed && generation == sourceGeneration) InstallSource = source;
        }
        catch (Exception ex) { if (!Closed && generation == sourceGeneration) SourceError = ex.Message; }
        finally { if (!Closed && generation == sourceGeneration) { SourceLoading = false; Notify(); } }
    }
    public async Task<bool> LoadFilesAsync()
    {
        if (Selected == null || Busy || Closed) return false;
        var name = S(Selected, "name"); Content = null;
        if (!await RunAsync(async () =>
        {
            var response = await client.InvokeAsync("list_skill_files", new() { ["name"] = name }, ProjectId);
            if (response is not JsonArray) throw new InvalidDataException("技能文件列表响应无效。");
            return Strings(response);
        }, files => { Files = files; File = files.FirstOrDefault(f => f == "SKILL.md") ?? files.FirstOrDefault(); }, ex => ex.Message)) return false;
        return File == null || await ReadAsync(File);
    }
    public Task<bool> ReadAsync(string path)
    {
        if (Busy || Closed || Selected == null || !Files.Contains(path, StringComparer.Ordinal)) return Task.FromResult(false);
        File = path; Content = null;
        return RunAsync(async () =>
        {
            var response = await client.InvokeAsync("read_skill_file", new() { ["name"] = S(Selected, "name"), ["path"] = path }, ProjectId);
            if (S(response, "path") != path || response?["content"] is not JsonValue) throw new InvalidDataException("技能文件响应与所选文件不一致。");
            return S(response, "content");
        }, content => Content = content, ex => ex.Message);
    }
    public void EditTags(string value) { if (!Busy && !Closed && Selected != null) { TagsText = value; Notify(); } }
    public Task<bool> SaveTagsAsync()
    {
        if (!TagsChanged || Selected == null) return Task.FromResult(false);
        var row = Selected; var tags = Tags(TagsText);
        return RunAsync(() => client.InvokeAsync("set_skill_tags", new() { ["name"] = S(row, "name"), ["tags"] = new JsonArray(tags.Select(t => (JsonNode?)JsonValue.Create(t)).ToArray()) }, ProjectId), _ =>
        { row["tags"] = new JsonArray(tags.Select(t => (JsonNode?)JsonValue.Create(t)).ToArray()); TagsText = string.Join(", ", tags); });
    }
    public Task<bool> SetEnabledAsync(string name, bool enabled) => SetEnabledAsync(Skills.Where(s => S(s, "name") == name && !B(s, "managed")).ToArray(), enabled);
    public Task<bool> SetVisibleEnabledAsync(bool enabled) => SetEnabledAsync(Visible.Where(s => !B(s, "managed")).ToArray(), enabled);
    private Task<bool> SetEnabledAsync(JsonObject[] rows, bool enabled)
    {
        if (ProjectId == null || rows.Length == 0) return Task.FromResult(false);
        // Older native hosts expose the single-skill command. Stop on the first
        // uncertain result and retain acknowledgements without replaying writes.
        return RunAsync(async () =>
        {
            foreach (var row in rows)
            {
                if (Closed) return false;
                await client.InvokeAsync("set_skill_enabled", new() { ["name"] = S(row, "name"), ["enabled"] = enabled }, ProjectId);
                if (!Closed) row["enabled"] = enabled;
            }
            return true;
        }, _ => { });
    }
    public async Task<bool> RemoveAsync()
    {
        if (Selected == null || !CanRemove(Selected) || TagsChanged || ScopeNeedsRead) return false;
        var name = S(Selected, "name");
        scopeNeedsRead.Add(name);
        if (!await RunAsync(() => client.InvokeAsync("remove_skill", new() { ["name"] = name }, ProjectId), _ => { })) return false;
        // A bundled, extra-path or plugin package may now provide the same name.
        return await RefreshScopeAsync();
    }
    public async Task<bool> InstallAsync(string path, string scope)
    {
        if (Busy || Closed || Selected != null || ProjectId == null || InstallLocations == null ||
            scope is not ("project" or "global") || string.IsNullOrWhiteSpace(path)) return false;
        InstallReceipt = null;
        var saved = await RunAsync(async () =>
        {
            // A host that predates scoped installs must reject this command; its
            // legacy install_skill silently ignores new arguments and installs globally.
            var result = await client.InvokeAsync("install_scoped_skill", new() { ["srcPath"] = path, ["scope"] = scope }, ProjectId);
            return result is JsonObject receipt && S(receipt, "scope") == scope &&
                !string.IsNullOrWhiteSpace(S(receipt, "name")) && !string.IsNullOrWhiteSpace(S(receipt, "dir"))
                ? receipt : throw new InvalidDataException("Invalid skill install receipt.");
        }, receipt => InstallReceipt = receipt);
        return saved && await LoadAsync();
    }
    public void Back() { if (Busy || Closed) return; ClearSelection(); ClearFailure(); Notify(); }
    private void ClearSelection()
    {
        PromotionReceipt = null; ProjectCopyRemoved = false; Selected = null; Files = []; File = Content = null; TagsText = ""; SourceMode = false;
        sourceGeneration++; InstallSource = null; SourceError = null; SourceLoading = false;
    }
    public static string MarkdownBody(string text)
    {
        var normalized = text.TrimStart('\uFEFF').Replace("\r\n", "\n");
        if (!normalized.StartsWith("---\n", StringComparison.Ordinal)) return normalized;
        var end = normalized.IndexOf("\n---\n", 4, StringComparison.Ordinal);
        return end < 0 ? normalized : normalized[(end + 5)..];
    }
    public override void Dispose() { base.Dispose(); Skills = []; ClearSelection(); }
}
