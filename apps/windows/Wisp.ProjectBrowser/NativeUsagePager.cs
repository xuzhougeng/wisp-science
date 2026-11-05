using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Read-only workspace drilldown. A new request or Back invalidates pending replies.
public sealed class NativeUsagePager(INativeSettingsClient client, string? context) : IDisposable
{
    public const int PageSize = 20;
    public event Action? Changed;
    public NativeUsageWorkspace? Selected { get; private set; }
    public IReadOnlyList<NativeUsageSession>? Items { get; private set; }
    public long Page { get; private set; }
    public long Total { get; private set; }
    public long PageCount => Math.Max(1, Total / PageSize + (Total % PageSize == 0 ? 0 : 1));
    public bool Loading { get; private set; }
    public string? Error { get; private set; }
    private long generation;
    private bool closed;
    private CancellationTokenSource? pending;
    public Task SelectAsync(NativeUsageWorkspace workspace)
    { if (closed) return Task.CompletedTask; Selected = workspace; Total = 0; return LoadAsync(0); }
    public Task GoAsync(long page) => closed || Selected == null || page < 0 || page >= PageCount ? Task.CompletedTask : LoadAsync(page);
    public Task RetryAsync() => closed || Selected == null ? Task.CompletedTask : LoadAsync(Page);
    public void Back()
    {
        if (closed) return;
        generation++; pending?.Cancel(); Selected = null; Items = null; Page = 0; Total = 0; Loading = false; Error = null; Changed?.Invoke();
    }
    private async Task LoadAsync(long page)
    {
        var workspace = Selected!; var current = ++generation;
        pending?.Cancel(); pending?.Dispose(); pending = new(); var token = pending.Token;
        Loading = true; Error = null; Items = null; Page = page; Changed?.Invoke();
        try
        {
            while (true)
            {
                var reply = await client.InvokeAsync("get_session_token_usage", new() { ["projectId"] = workspace.Id, ["offset"] = checked(page * PageSize), ["limit"] = PageSize }, context, token);
                if (closed || current != generation) return;
                if (reply is not JsonObject data || data["items"] is not JsonArray || data["total"] is not JsonValue count || !count.TryGetValue<long>(out var total)) throw new InvalidDataException("Missing session usage page.");
                Total = Math.Max(0, total);
                if (page > 0 && page >= PageCount) { Page = page = PageCount - 1; continue; }
                Items = NativeUsagePresentation.Sessions(data); break;
            }
        }
        catch (Exception ex) { if (!closed && current == generation) Error = ex.Message; }
        finally { if (!closed && current == generation) { Loading = false; Changed?.Invoke(); } }
    }
    public void Dispose() { closed = true; generation++; pending?.Cancel(); pending?.Dispose(); pending = null; Selected = null; Items = null; Loading = false; Changed = null; }
}
