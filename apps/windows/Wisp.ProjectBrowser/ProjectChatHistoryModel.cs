using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class ProjectChatHistoryModel(IProjectChatHistory client, string database, string project) : IDisposable
{
    public event Action? Changed;
    public string Query { get; private set; } = "";
    public IReadOnlyList<ProjectChatHit> Items { get; private set; } = [];
    public ProjectChatCursor? Next { get; private set; }
    public bool Loading { get; private set; }
    public string? Error { get; private set; }
    private CancellationTokenSource? pending;
    private bool closed;

    public Task EditAsync(string query, TimeSpan delay)
    {
        if (closed) return Task.CompletedTask;
        pending?.Cancel();
        Query = query; Items = []; Next = null; Error = null;
        if (!string.IsNullOrWhiteSpace(query)) return FetchAsync(null, delay);
        pending = null; Loading = false; Changed?.Invoke(); return Task.CompletedTask;
    }
    public Task ContinueAsync() => closed || Loading || string.IsNullOrWhiteSpace(Query) || Next == null && Error == null
        ? Task.CompletedTask : FetchAsync(Next, TimeSpan.Zero);
    private async Task FetchAsync(ProjectChatCursor? cursor, TimeSpan delay)
    {
        using var request = new CancellationTokenSource();
        pending = request; Loading = true; Error = null; Changed?.Invoke();
        try
        {
            await Task.Delay(delay, request.Token);
            var page = await client.SearchChatsAsync(database, project, Query, cursor, request.Token);
            if (closed || pending != request) return;
            Items = cursor == null ? page.Items : [.. Items, .. page.Items];
            Next = page.NextCursor;
        }
        catch (OperationCanceledException) when (request.IsCancellationRequested) { }
        catch (Exception e) { if (!closed && pending == request) Error = e.Message; }
        finally { if (!closed && pending == request) { pending = null; Loading = false; Changed?.Invoke(); } }
    }
    public void Dispose() { closed = true; pending?.Cancel(); pending = null; }
}
