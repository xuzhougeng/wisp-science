using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>A search belongs to one overlay/database/project preference. Late
/// connections and responses cannot refill an edited or closed overlay.</summary>
public sealed class WorkspaceSearchModel(Func<Task<INativeSearchClient?>> connect, string? preferredProject) : IDisposable
{
    public event Action? Changed;
    public NativeSearchItem[] Items { get; private set; } = [];
    public string Query { get; private set; } = "";
    public string? Error { get; private set; }
    public bool Busy { get; private set; }
    private bool disposed;
    private int generation;
    private CancellationTokenSource? pending;

    public async Task SearchAsync(string query, TimeSpan? debounce = null)
    {
        if (disposed) return;
        var current = ++generation;
        pending?.Cancel(); pending?.Dispose();
        using var request = new CancellationTokenSource(); pending = request;
        Query = query; Items = []; Error = null; Busy = true; Changed?.Invoke();
        try
        {
            if (System.Text.Encoding.UTF8.GetByteCount(query) > 512) throw new InvalidOperationException("搜索关键词过长，请缩短后重试。");
            if (debounce is { } delay) await Task.Delay(delay, request.Token);
            var client = await connect() ?? throw new InvalidOperationException("搜索服务不可用，请确认原生主机已运行并支持工作区搜索。");
            if (disposed || current != generation) return;
            var result = await client.SearchAsync(query, preferredProject, request.Token);
            if (disposed || current != generation) return;
            if (result.Query != query || result.PreferredProjectId != preferredProject || result.Schema != NativeSearchClient.Schema
                || result.Items == null || result.Items.Any(item => !item.Valid)) throw new InvalidDataException("搜索结果与当前范围不符。");
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
