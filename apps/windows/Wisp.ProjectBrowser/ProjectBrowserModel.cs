using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>UI-thread state with independent refresh, navigation and transcript generations.</summary>
public sealed class ProjectBrowserModel(IProjectBrowserClient client, string databasePath) : IDisposable
{
    public event Action? Changed;
    public string DatabasePath { get; private set; } = databasePath;
    public IReadOnlyList<ProjectSummary> Projects { get; private set; } = [];
    public IReadOnlyList<BrowserSession> RecentSessions { get; private set; } = [];
    public IReadOnlyList<BrowserSession> Sessions { get; private set; } = [];
    public IReadOnlyList<BrowserMessage> Messages { get; private set; } = [];
    public string? ActiveProjectId { get; private set; }
    public string? ActiveSessionId { get; private set; }
    public long? NextBeforeSeq { get; private set; }
    public bool Loading { get; private set; }
    public bool SessionsLoading { get; private set; }
    public bool TranscriptLoading { get; private set; }
    public string? Error { get; private set; }
    public string? SessionError { get; private set; }
    public DateTimeOffset? LastLoaded { get; private set; }
    public int NavigationRevision => navigationGeneration;
    private int refreshGeneration, navigationGeneration, transcriptGeneration;
    private readonly CancellationTokenSource lifetime = new();

    // Update only the accepted live conversation. The window refreshes labels
    // in place; a status transition must not reload navigation or the composer.
    public bool ApplySessionActivity(ConversationSnapshot snapshot)
    {
        if (snapshot.ProjectId != ActiveProjectId || snapshot.SessionId != ActiveSessionId
            || snapshot.ActivityStatus is not ("running" or "needs_you" or "complete")) return false;
        var changed = false;
        BrowserSession Update(BrowserSession row)
        {
            if (row.ProjectId != snapshot.ProjectId || row.Id != snapshot.SessionId || row.Status == snapshot.ActivityStatus) return row;
            changed = true; return row with { Status = snapshot.ActivityStatus };
        }
        Sessions = Sessions.Select(Update).ToArray();
        RecentSessions = RecentSessions.Select(Update).ToArray();
        return changed;
    }

    public async Task RefreshAsync()
    {
        if (Loading) return;
        var generation = ++refreshGeneration;
        var navigation = navigationGeneration;
        Loading = true;
        Error = null;
        Notify();
        try
        {
            var snapshot = await client.ListProjectsAsync(DatabasePath, lifetime.Token);
            var recent = await client.ListSessionsAsync(DatabasePath, cancellationToken: lifetime.Token);
            if (generation != refreshGeneration) return;
            Projects = snapshot.Projects;
            RecentSessions = recent.Take(5).ToArray();
            LastLoaded = DateTimeOffset.Now;
            if (navigation == navigationGeneration && ActiveProjectId is { } id)
            {
                if (Projects.Any(p => p.Id == id)) await OpenProjectAsync(id, ActiveSessionId);
                else GoHome();
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (generation == refreshGeneration) Error = ex.Message;
        }
        catch (OperationCanceledException) { }
        finally { if (generation == refreshGeneration) { Loading = false; Notify(); } }
    }

    public async Task SetStarredAsync(string projectId, bool starred)
    {
        if (Loading || !Projects.Any(p => p.Id == projectId)) return;
        var generation = ++refreshGeneration;
        Loading = true; Error = null; Notify();
        try
        {
            var snapshot = await client.SetProjectStarredAsync(DatabasePath, projectId, starred, lifetime.Token);
            if (generation == refreshGeneration) { Projects = snapshot.Projects; LastLoaded = DateTimeOffset.Now; }
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (generation == refreshGeneration) Error = ex.Message; }
        finally { if (generation == refreshGeneration) { Loading = false; Notify(); } }
    }

    public async Task ChangeDatabaseAsync(string path)
    {
        ++refreshGeneration;
        Loading = false;
        DatabasePath = path;
        Projects = [];
        RecentSessions = [];
        LastLoaded = null;
        Error = null;
        GoHome();
        await RefreshAsync();
    }

    public void GoHome()
    {
        ++navigationGeneration;
        ++transcriptGeneration;
        ActiveProjectId = ActiveSessionId = null;
        Sessions = [];
        Messages = [];
        NextBeforeSeq = null;
        SessionsLoading = TranscriptLoading = false;
        SessionError = null;
        Notify();
    }

    public async Task OpenProjectAsync(string id, string? sessionId = null)
    {
        if (!Projects.Any(p => p.Id == id)) return;
        var generation = ++navigationGeneration;
        ++transcriptGeneration;
        ActiveProjectId = id;
        ActiveSessionId = sessionId;
        Sessions = [];
        Messages = [];
        NextBeforeSeq = null;
        TranscriptLoading = false;
        SessionsLoading = true;
        SessionError = null;
        Notify();
        try
        {
            var rows = await client.ListSessionsAsync(DatabasePath, id, lifetime.Token);
            if (generation != navigationGeneration) return;
            Sessions = rows.Where(s => s.ProjectId == id).ToArray();
            if (sessionId != null && !Sessions.Any(s => s.Id == sessionId)) SessionError = "这个会话已不存在，请刷新项目列表。";
            else if ((sessionId ?? Sessions.FirstOrDefault()?.Id) is { } selected) await OpenSessionAsync(selected);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (generation == navigationGeneration) SessionError = ex.Message;
        }
        catch (OperationCanceledException) { }
        finally { if (generation == navigationGeneration) { SessionsLoading = false; Notify(); } }
    }

    public async Task OpenSessionAsync(string id, bool older = false)
    {
        if (ActiveProjectId is not { } projectId || !Sessions.Any(s => s.Id == id)) return;
        if (older && (TranscriptLoading || id != ActiveSessionId || NextBeforeSeq == null)) return;
        var generation = ++transcriptGeneration;
        var cursor = older ? NextBeforeSeq : null;
        ActiveSessionId = id;
        if (!older) { Messages = []; NextBeforeSeq = null; }
        TranscriptLoading = true;
        SessionError = null;
        Notify();
        try
        {
            var page = await client.GetTranscriptAsync(DatabasePath, projectId, id, cursor, lifetime.Token);
            if (generation != transcriptGeneration) return;
            Messages = (older ? page.Messages.Concat(Messages) : page.Messages).DistinctBy(m => m.Sequence).OrderBy(m => m.Sequence).ToArray();
            NextBeforeSeq = page.NextBeforeSeq;
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (generation == transcriptGeneration) SessionError = ex.Message;
        }
        catch (OperationCanceledException) { }
        finally { if (generation == transcriptGeneration) { TranscriptLoading = false; Notify(); } }
    }

    public IEnumerable<SearchResult> Search(string query)
    {
        bool Matches(string value) => value.Contains(query.Trim(), StringComparison.OrdinalIgnoreCase);
        if (ActiveProjectId == null)
            foreach (var p in Projects.Where(p => Matches(p.Name) || Matches(p.Description) || Matches(p.WorkspaceDirectory)))
                yield return new(p.Name, p.WorkspaceDirectory, p.Id, null);
        foreach (var s in (ActiveProjectId == null ? RecentSessions : Sessions).Where(s => Matches(s.Title)))
            yield return new(s.Title, Projects.FirstOrDefault(p => p.Id == s.ProjectId)?.Name ?? "会话", s.ProjectId, s.Id);
    }

    private void Notify() { if (!lifetime.IsCancellationRequested) Changed?.Invoke(); }
    public void Dispose() { lifetime.Cancel(); lifetime.Dispose(); }
}

public sealed record SearchResult(string Title, string Detail, string ProjectId, string? SessionId);
