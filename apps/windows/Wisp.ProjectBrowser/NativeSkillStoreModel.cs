using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;
using static Wisp.ProjectBrowser.NativeSkillsModel;

namespace Wisp.ProjectBrowser;

/// Preview is cancellable browsing; installation submits only the reviewed immutable source.
public sealed class NativeSkillStoreModel(INativeSettingsClient client, string? projectId, bool githubFirst, NativeLocalization? localization = null) : WorkspaceActionModel
{
    public static readonly (string Name, string Url, string DescriptionKey)[] Marketplaces =
    [
        ("OpenAI Skills", "https://github.com/openai/skills/tree/main/skills/.curated", "store.openai_source"),
        ("Anthropic Skills", "https://github.com/anthropics/skills/tree/main/skills", "store.anthropic_source"),
        ("BEAR Research Skills", "https://github.com/fei0810/bear-research-skills/tree/main/skills", "store.bear_source")
    ];
    public IReadOnlyList<JsonObject> Catalog { get; private set; } = [];
    public IReadOnlyList<JsonObject> Candidates { get; private set; } = [];
    public HashSet<string> InstalledNames { get; private set; } = new(StringComparer.OrdinalIgnoreCase);
    public string? CatalogNotice { get; private set; }
    public string? PreviewError { get; private set; }
    public string? Success { get; private set; }
    public bool Github { get; private set; } = githubFirst;
    public int? Marketplace { get; private set; }
    public JsonObject? SelectedEntry { get; private set; }
    public JsonObject? Selected { get; private set; }
    public JsonObject? ConfirmationSource { get; private set; }
    public bool Confirming => ConfirmationSource != null;
    public bool Previewing { get; private set; }
    public bool Previewed { get; private set; }
    public string SourceUrl { get; private set; } = "";
    public string ExactRef { get; private set; } = "";
    public string Query { get; set; } = "";
    public string Tag { get; set; } = "";
    public string CandidateQuery { get; set; } = "";
    private long previewGeneration;
    public long PreviewVersion => previewGeneration;
    private CancellationTokenSource? previewCancellation;
    private static JsonObject[] Rows(JsonNode? value) => value is JsonArray array && array.All(row => row is JsonObject)
        ? array.OfType<JsonObject>().Select(row => (JsonObject)row.DeepClone()).ToArray() : throw new InvalidDataException("技能商店响应格式无效。");
    public JsonObject[] Visible => Catalog.Where(entry =>
        (Tag.Length == 0 || Strings(entry["tags"]).Contains(Tag, StringComparer.Ordinal)) && MatchesText(entry, Query, "name", "description", "author", "repository")).ToArray();
    public JsonObject[] VisibleCandidates => Candidates.Where(entry => MatchesText(entry, CandidateQuery, "name", "description")).ToArray();
    private static bool MatchesText(JsonObject entry, string query, params string[] fields) =>
        (string.Join(" ", fields.Select(f => S(entry, f))) + " " + string.Join(" ", Strings(entry["tags"]))).Contains(query.Trim(), StringComparison.OrdinalIgnoreCase);
    public string[] AllTags => Catalog.SelectMany(entry => Strings(entry["tags"])).Distinct(StringComparer.Ordinal).Order(StringComparer.Ordinal).ToArray();
    public static string MetadataValue(JsonObject entry, string field, NativeLocalization? locale = null) => entry[field] is JsonArray
        ? string.Join("; ", Strings(entry[field]))
        : S(entry, field).Length > 0 ? S(entry, field)
        : field == "verified_wisp" ? locale?.Text("store.unverified") ?? "本次安装未验证实际运行效果。" : locale?.Choose("未提供", "Not provided") ?? "未提供";
    public static string CandidateStatus(JsonObject candidate, NativeLocalization? locale = null)
    {
        var (key, fallback) = candidate["conflict"] != null ? ("store.conflict", "同名冲突，保留现有文件")
            : candidate["installed_source"] != null ? ("store.installed", "已安装")
            : Strings(candidate["format_errors"]).Length > 0 || Strings(candidate["resource_errors"]).Length > 0 ? ("store.invalid", "校验未通过")
            : ("store.format_pass", "格式校验通过");
        return locale?.Text(key) ?? fallback;
    }
    public static bool Installable(JsonObject candidate) => candidate["source"] is JsonObject source && S(source, "repository").Length > 0 && S(source, "commit").Length > 0
        && candidate["conflict"] == null && candidate["installed_source"] == null
        && candidate["format_errors"] is JsonArray { Count: 0 } && candidate["resource_errors"] is JsonArray { Count: 0 };
    public Task<bool> LoadCatalogAsync(bool refresh = false) => RunAsync(async () =>
    {
        var response = await client.InvokeAsync("list_community_skills", new() { ["refresh"] = refresh });
        var entries = Rows(response?["entries"]); var notice = S(response, "notice");
        var installed = projectId == null ? [] : Rows(await client.InvokeAsync("list_skills", new(), projectId));
        return (entries, notice, installed);
    }, result => { Catalog = result.entries; CatalogNotice = result.notice; InstalledNames = result.installed.Select(row => S(row, "name")).ToHashSet(StringComparer.OrdinalIgnoreCase); });
    public void EditSource(string url, string exactRef)
    {
        if (Busy || Closed || Previewing || Confirming) return;
        SourceUrl = url; ExactRef = exactRef; ClearPreview(); Notify();
    }
    public void SetMode(bool github)
    {
        if (Busy || Closed || Previewing || Confirming) return;
        Github = github; Marketplace = null; SelectedEntry = null; ClearPreview(); ClearFailure(); Notify();
    }
    private static string EncodePath(string path) => string.Join('/', path.Split('/').Select(Uri.EscapeDataString));
    public Task<bool> PreviewEntryAsync(JsonObject entry)
    {
        if (Busy || Closed || Previewing || Confirming || !Catalog.Contains(entry)) return Task.FromResult(false);
        SelectedEntry = (JsonObject)entry.DeepClone(); Marketplace = null; Github = false;
        ExactRef = S(entry, "git_ref"); SourceUrl = $"https://github.com/{S(entry, "repository")}/tree/{EncodePath((ExactRef + "/" + S(entry, "package_path")).TrimEnd('/'))}";
        return PreviewAsync();
    }
    public Task<bool> PreviewMarketplaceAsync(int index)
    {
        if (Busy || Closed || Previewing || Confirming || index < 0 || index >= Marketplaces.Length) return Task.FromResult(false);
        Marketplace = index; Github = false; SelectedEntry = null; SourceUrl = Marketplaces[index].Url; ExactRef = "main";
        return PreviewAsync();
    }
    public async Task<bool> PreviewAsync()
    {
        if (Busy || Closed || Previewing || Confirming || SourceUrl.Trim().Length == 0 || projectId == null) return false;
        ClearPreview(); ClearFailure(); var generation = ++previewGeneration;
        var cancellation = new CancellationTokenSource(); previewCancellation = cancellation;
        var url = SourceUrl; var exactRef = ExactRef; Previewing = true; Notify();
        try
        {
            var response = await client.InvokeAsync("preview_github_skills", new() { ["sourceUrl"] = url, ["exactRef"] = exactRef }, projectId, cancellation.Token);
            if (Closed || generation != previewGeneration) return false;
            Candidates = Rows(response); Previewed = true; return true;
        }
        catch (Exception ex) { if (!Closed && generation == previewGeneration) PreviewError = ex.Message; return false; }
        finally
        {
            if (generation == previewGeneration && !Closed) { Previewing = false; previewCancellation = null; Notify(); }
            cancellation.Dispose();
        }
    }
    public void CancelPreview()
    {
        if (Busy || Closed) return;
        previewGeneration++; previewCancellation?.Cancel(); previewCancellation = null; Previewing = false; ClearPreview(); Notify();
    }
    private void ClearPreview() { Candidates = []; Selected = null; ConfirmationSource = null; CandidateQuery = ""; PreviewError = Success = null; Previewed = false; }
    public bool Select(JsonObject candidate)
    {
        if (Busy || Closed || Previewing || Confirming || !Candidates.Contains(candidate)) return false;
        Selected = candidate; Success = null; PreviewError = null; ClearFailure(); Notify(); return true;
    }
    public bool BeginConfirmation()
    {
        if (Busy || Closed || Previewing || Selected == null || !Installable(Selected)) return false;
        ConfirmationSource = (JsonObject)Selected["source"]!.DeepClone(); ClearFailure(); Notify(); return true;
    }
    /// Returns false only when the owning settings page should close the store.
    public bool Back()
    {
        if (Busy || Closed) return true;
        if (Confirming) { ConfirmationSource = null; ClearFailure(); }
        else if (Selected != null) { Selected = null; Success = null; ClearFailure(); }
        else if (Previewing) { CancelPreview(); return true; }
        else if (Marketplace != null) { Marketplace = null; ClearPreview(); }
        else return false;
        Notify(); return true;
    }
    public Task<bool> InstallAsync()
    {
        if (!Confirming || Selected == null || !Installable(Selected) || !JsonNode.DeepEquals(Selected["source"], ConfirmationSource)) return Task.FromResult(false);
        var source = ConfirmationSource!.DeepClone(); var candidate = Selected; var name = S(candidate, "name");
        return RunAsync(async () =>
        {
            var result = await client.InvokeAsync("install_github_skill", new() { ["source"] = source }, projectId);
            if (S(result, "name") != name || S(result, "directory").Length == 0) throw new InvalidDataException("安装响应与已确认的技能包不一致。请刷新检查结果。");
            return result!;
        }, result =>
        {
            ConfirmationSource = null; candidate["installed_source"] = source.DeepClone(); candidate["conflict"] = null;
            InstalledNames.Add(name); Success = (localization?.Text("store.installed") ?? "已安装") + ": " + S(result, "directory") + (S(result, "notice").Length > 0 ? "\n" + S(result, "notice") : "");
        });
    }
    public override void Dispose()
    {
        previewGeneration++; previewCancellation?.Cancel(); previewCancellation = null; Previewing = false;
        base.Dispose(); Catalog = []; ClearPreview();
    }
}
