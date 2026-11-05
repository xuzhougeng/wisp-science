using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>A project-scoped collection with keyset paging. Closing or changing the search
/// invalidates old reads. Restores are acknowledged once and never automatically replayed.</summary>
public sealed class WorkspaceShelvedSessions(INativeSettingsClient client, string projectId) : IDisposable
{
    public event Action? Changed;
    public string Query { get; private set; } = "";
    public ShelvedSession[] Items { get; private set; } = [];
    public ShelvedSessionCursor? NextCursor { get; private set; }
    public bool Loading { get; private set; }
    public string? RestoringId { get; private set; }
    public string? Error { get; private set; }
    public bool RestoreUnconfirmed { get; private set; }
    public bool DidRestore { get; private set; }
    private readonly HashSet<string> restored = [];
    private readonly CancellationTokenSource lifetime = new();
    private int generation;
    private bool closed;

    public Task SearchAsync(string query) => ReadAsync(query, false);
    public Task MoreAsync() => Loading || NextCursor == null ? Task.CompletedTask : ReadAsync(Query, true);
    private async Task ReadAsync(string query, bool more)
    {
        if (closed) return;
        var current = ++generation;
        var cursor = more ? NextCursor : null;
        Query = query; Loading = true;
        if (!RestoreUnconfirmed) Error = null;
        if (!more) { Items = []; NextCursor = null; }
        Changed?.Invoke();
        try
        {
            var args = new JsonObject { ["query"] = query,
                ["cursor"] = cursor == null ? null : JsonSerializer.SerializeToNode(cursor) };
            var data = await client.InvokeAsync("native_project_shelved_sessions", args, projectId, lifetime.Token);
            if (closed || current != generation) return;
            var page = data?.Deserialize<ShelvedSessionPage>() ?? throw new InvalidDataException("Missing shelved conversations");
            if (page.ProjectId != projectId || page.Items == null || page.Items.Any(s => string.IsNullOrWhiteSpace(s.Id) || s.Title == null)
                || page.NextCursor is { } next && (string.IsNullOrWhiteSpace(next.Id) || next == cursor))
                throw new InvalidDataException("Invalid shelved conversation page");
            Items = (more ? Items.Concat(page.Items) : page.Items).Where(s => !restored.Contains(s.Id)).DistinctBy(s => s.Id).ToArray();
            NextCursor = page.NextCursor;
        }
        catch (Exception ex) { if (!closed && current == generation) Error = ex.Message; }
        finally { if (!closed && current == generation) { Loading = false; Changed?.Invoke(); } }
    }
    public async Task<bool> RestoreAsync(string sessionId)
    {
        if (closed || RestoringId != null || RestoreUnconfirmed || !Items.Any(s => s.Id == sessionId)) return false;
        RestoringId = sessionId; Error = null; Changed?.Invoke();
        try
        {
            var data = await client.InvokeAsync("native_project_session_shelve", new() { ["session_id"] = sessionId, ["shelved"] = false }, projectId, lifetime.Token);
            if (closed) return false;
            var result = data?.Deserialize<SessionShelveResult>();
            if (result == null || result.ProjectId != projectId || result.SessionId != sessionId || result.Shelved)
                throw new InvalidDataException("Restore acknowledgement did not match the conversation");
            restored.Add(sessionId); Items = Items.Where(s => s.Id != sessionId).ToArray(); DidRestore = true;
            return true;
        }
        catch (Exception ex) { if (!closed) { Error = ex.Message; RestoreUnconfirmed = true; } return false; }
        finally { if (!closed) { RestoringId = null; Changed?.Invoke(); } }
    }
    public void Dispose() { if (closed) return; closed = true; generation++; lifetime.Cancel(); lifetime.Dispose(); Changed = null; }
}
