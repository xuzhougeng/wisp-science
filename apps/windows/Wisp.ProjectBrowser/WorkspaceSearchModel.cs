using Wisp.ProjectBrowser.Contracts;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// <summary>A search belongs to one overlay/database/project preference. Late
/// connections and responses cannot refill an edited or closed overlay.</summary>
public sealed class WorkspaceSearchModel(Func<Task<INativeSearchClient?>> connect, string? preferredProject, NativeLocalization? locale = null) : IDisposable
{
    public event Action? Changed;
    public NativeSearchItem[] Items { get; private set; } = [];
    public string Query { get; private set; } = "";
    public string? Error { get; private set; }
    public bool Busy { get; private set; }
    public string? CommandProjectId { get; private set; } = preferredProject;
    private readonly string? searchProjectId = preferredProject;
    private bool commandProjectRequested;
    private bool disposed;
    private int generation;
    private CancellationTokenSource? pending;
    private string L(string zh, string en) => locale?.Choose(zh, en) ?? en;

    public async Task LoadCommandProjectAsync(INativeSettingsClient client, IReadOnlyCollection<(string Id, string Root)> visibleProjects)
    {
        if (disposed || CommandProjectId != null || commandProjectRequested) return;
        commandProjectRequested = true;
        try
        {
            // Home keeps the host's restored workspace for settings, just like
            // WebView. Never infer a target from project list order or search hits.
            // Unscoped native command windows deliberately have no active
            // project, so get_project_info cannot resolve this on Home.
            var info = await client.InvokeAsync("get_bootstrap_status", new());
            if (disposed || info is not JsonObject || info["workspace"] is not JsonValue value
                || !value.TryGetValue<string>(out var root) || string.IsNullOrWhiteSpace(root)) return;
            static string Normalize(string path) => path.Replace('\\', '/').TrimEnd('/');
            var normalized = Normalize(root);
            var comparison = normalized.Length > 2 && normalized[1] == ':' || normalized.StartsWith("//")
                ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal;
            var matches = visibleProjects.Where(project => string.Equals(Normalize(project.Root), normalized, comparison)).ToArray();
            if (matches.Length == 1) { CommandProjectId = matches[0].Id; Changed?.Invoke(); }
        }
        catch { } // Optional settings shortcuts must not make document search fail.
    }

    public async Task SearchAsync(string query, TimeSpan? debounce = null)
    {
        if (disposed) return;
        var current = ++generation;
        pending?.Cancel(); pending?.Dispose();
        using var request = new CancellationTokenSource(); pending = request;
        Query = query; Items = []; Error = null; Busy = true; Changed?.Invoke();
        try
        {
            if (System.Text.Encoding.UTF8.GetByteCount(query) > 512) throw new InvalidOperationException(L("搜索关键词过长，请缩短后重试。", "Search query is too long. Shorten it and try again."));
            if (debounce is { } delay) await Task.Delay(delay, request.Token);
            var client = await connect() ?? throw new InvalidOperationException(L("搜索服务不可用，请确认原生主机已运行并支持工作区搜索。", "Search is unavailable. Check that the desktop host is running and supports workspace search."));
            if (disposed || current != generation) return;
            var result = await client.SearchAsync(query, searchProjectId, request.Token);
            if (disposed || current != generation) return;
            if (result.Query != query || result.PreferredProjectId != searchProjectId || result.Schema != NativeSearchClient.Schema
                || result.Items == null || result.Items.Any(item => !item.Valid)) throw new InvalidDataException(L("搜索结果与当前范围不符。", "Search results do not match this workspace."));
            Items = result.Items;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!disposed && current == generation) Error = ex.Message; }
        finally
        {
            if (ReferenceEquals(pending, request)) pending = null;
            if (!disposed && current == generation) { Busy = false; Changed?.Invoke(); }
        }
    }
    public void Dispose() { disposed = true; generation++; pending?.Cancel(); pending = null; Items = []; Busy = false; }
}
