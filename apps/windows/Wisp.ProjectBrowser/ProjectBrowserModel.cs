using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>UI-thread state with independent refresh, navigation and transcript generations.</summary>
public sealed class ProjectBrowserModel(IProjectBrowserClient client, string databasePath) : IDisposable
{
    public event Action? Changed;
    public string DatabasePath { get; private set; } = databasePath;
    public IReadOnlyList<ProjectSummary> Projects { get; private set; } = [];
    private HashSet<string>? hiddenHomeProjects;
    private int privacyRevision;
    public int PrivacyRevision => privacyRevision;
    public Task<ProjectListSnapshot> PrivacyChoicesAsync() => client.ListProjectsAsync(DatabasePath, lifetime.Token);
    private Dictionary<string, HomeActivityCount>? liveCounts;
    private BrowserSession[]? liveRecent;
    public bool HomeActivitySavedOnly { get; private set; }
    public IReadOnlyList<ProjectSummary> HomeProjects => Projects.Where(p => HomeVisible(p.Id)).Select(p =>
        liveCounts?.TryGetValue(p.Id, out var count) == true ? p with { RunningCount = count.Running, NeedsYouCount = count.NeedsYou } : p).ToArray();
    public IReadOnlyList<BrowserSession> HomeRecentSessions => (liveRecent ?? RecentSessions).Where(s => HomeVisible(s.ProjectId)).ToArray();
    public event Action? HomeActivityChanged;
    public void ApplyHomeActivity(HomeActivitySnapshot? snapshot)
    {
        if (ActiveProjectId != null || Loading || LastLoaded == null) return;
        var before = HomeProjects; var recent = HomeRecentSessions; var wasSavedOnly = HomeActivitySavedOnly;
        HomeActivitySavedOnly = snapshot == null;
        var visibility = hiddenHomeProjects;
        if (snapshot != null) SetPrivacy(snapshot.Privacy);
        liveCounts = snapshot?.Projects.ToDictionary(p => p.ProjectId);
        var known = Projects.Select(p => p.Id).ToHashSet();
        liveRecent = snapshot?.Recent.Where(s => known.Contains(s.ProjectId)).ToArray();
        if (snapshot != null && (visibility == null || !visibility.SetEquals(hiddenHomeProjects!)))
        { ++privacyRevision; Notify(); }
        else if (wasSavedOnly != HomeActivitySavedOnly || !before.SequenceEqual(HomeProjects) || !recent.SequenceEqual(HomeRecentSessions))
            HomeActivityChanged?.Invoke();
    }
    private bool HomeVisible(string id) => hiddenHomeProjects != null && !hiddenHomeProjects.Contains(id);
    public void ApplyPrivacy(PrivacyMode? mode)
    {
        ++privacyRevision;
        var before = hiddenHomeProjects;
        SetPrivacy(mode);
        if (before == null ? hiddenHomeProjects != null : hiddenHomeProjects == null || !before.SetEquals(hiddenHomeProjects))
        { liveCounts = null; liveRecent = null; Notify(); }
    }
    private void SetPrivacy(PrivacyMode? mode) => hiddenHomeProjects = mode is { ProjectIds: not null }
        ? new HashSet<string>(mode.Active ? mode.ProjectIds : [], StringComparer.Ordinal) : null;
    public IReadOnlyList<BrowserSession> RecentSessions { get; private set; } = [];
    public IReadOnlyList<BrowserSession> Sessions { get; private set; } = [];
    public IReadOnlyList<BrowserMessage> Messages { get; private set; } = [];
    public string? ActiveProjectId { get; private set; }
    public string? ActiveSessionId { get; private set; }
    private BrowserSession? shelvedSession;
    private BrowserSession? explorationSession;
    private BrowserSession? createdDraft;
    private int explorationOpenGeneration;
    public BrowserSession? ActiveSession => Sessions.FirstOrDefault(s => s.Id == ActiveSessionId)
        ?? (shelvedSession?.Id == ActiveSessionId ? shelvedSession : null)
        ?? (explorationSession?.Id == ActiveSessionId ? explorationSession : null);
    public long? NextBeforeSeq { get; private set; }
    public bool Loading { get; private set; }
    public bool SessionsLoading { get; private set; }
    public bool TranscriptLoading { get; private set; }
    public string? Error { get; private set; }
    public string? SessionError { get; private set; }
    public DateTimeOffset? LastLoaded { get; private set; }
    public int NavigationRevision => navigationGeneration;
    public int SessionNavigationRevision => explorationOpenGeneration;
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
        if (shelvedSession != null) shelvedSession = Update(shelvedSession);
        if (explorationSession != null) explorationSession = Update(explorationSession);
        RecentSessions = RecentSessions.Select(Update).ToArray();
        return changed;
    }

    public async Task RefreshAsync(bool selectFirst = true)
    {
        if (Loading) return;
        var generation = ++refreshGeneration;
        var readPrivacyRevision = privacyRevision;
        var navigation = navigationGeneration;
        Loading = true;
        Error = null;
        Notify();
        try
        {
            var snapshot = await client.ListProjectsAsync(DatabasePath, lifetime.Token);
            var recent = await client.ListSessionsAsync(DatabasePath, cancellationToken: lifetime.Token);
            if (generation != refreshGeneration) return;
            if (readPrivacyRevision == privacyRevision) SetPrivacy(snapshot.Privacy);
            Projects = snapshot.Projects;
            RecentSessions = recent.Take(5).ToArray();
            liveCounts = null; liveRecent = null;
            LastLoaded = DateTimeOffset.Now;
            if (navigation == navigationGeneration && ActiveProjectId is { } id)
            {
                if (Projects.Any(p => p.Id == id)) await OpenProjectAsync(id, ActiveSessionId, selectFirst);
                else GoHome();
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (generation == refreshGeneration) { Error = ex.Message; if (privacyRevision == readPrivacyRevision) SetPrivacy(null); }
        }
        catch (OperationCanceledException) { }
        finally { if (generation == refreshGeneration) { Loading = false; Notify(); } }
    }

    // A first launch can query the read-only service before the desktop host
    // creates its database. Recover that failed read once the same host is ready,
    // without reloading a populated browser or following a stale connection.
    public Task RefreshAfterHostReadyAsync(string database)
    {
        if (lifetime.IsCancellationRequested || database != DatabasePath || Loading
            || LastLoaded != null || Error == null) return Task.CompletedTask;
        return RefreshAsync();
    }

    public async Task SetStarredAsync(string projectId, bool starred)
    {
        if (Loading || !Projects.Any(p => p.Id == projectId)) return;
        var generation = ++refreshGeneration;
        var readPrivacyRevision = privacyRevision;
        Loading = true; Error = null; Notify();
        try
        {
            var snapshot = await client.SetProjectStarredAsync(DatabasePath, projectId, starred, lifetime.Token);
            if (generation == refreshGeneration) { if (readPrivacyRevision == privacyRevision) SetPrivacy(snapshot.Privacy); Projects = snapshot.Projects; LastLoaded = DateTimeOffset.Now; }
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (generation == refreshGeneration) { Error = ex.Message; if (privacyRevision == readPrivacyRevision) SetPrivacy(null); } }
        finally { if (generation == refreshGeneration) { Loading = false; Notify(); } }
    }

    public async Task ChangeDatabaseAsync(string path)
    {
        ++refreshGeneration;
        Loading = false;
        ++privacyRevision; SetPrivacy(null);
        DatabasePath = path;
        HomeActivitySavedOnly = false;
        Projects = [];
        RecentSessions = [];
        LastLoaded = null;
        Error = null;
        GoHome();
        await RefreshAsync();
    }

    public void GoHome()
    {
        liveCounts = null; liveRecent = null;
        ++explorationOpenGeneration; explorationSession = null;
        ++navigationGeneration;
        ++transcriptGeneration;
        ActiveProjectId = ActiveSessionId = null;
        shelvedSession = null;
        createdDraft = null;
        Sessions = [];
        Messages = [];
        NextBeforeSeq = null;
        SessionsLoading = TranscriptLoading = false;
        SessionError = null;
        Notify();
    }

    public async Task OpenProjectAsync(string id, string? sessionId = null, bool selectFirst = true)
    {
        if (!Projects.Any(p => p.Id == id)) return;
        ++explorationOpenGeneration;
        if (explorationSession?.ProjectId != id || explorationSession.Id != sessionId) explorationSession = null;
        if (shelvedSession?.ProjectId != id || shelvedSession.Id != sessionId) shelvedSession = null;
        if (createdDraft?.ProjectId != id || createdDraft.Id != sessionId) createdDraft = null;
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
            if (createdDraft is { } draft)
            {
                if (Sessions.Any(s => s.Id == draft.Id)) createdDraft = null;
                else Sessions = Sessions.Append(draft).ToArray();
            }
            if (sessionId != null && !Sessions.Any(s => s.Id == sessionId) && shelvedSession?.Id != sessionId && explorationSession?.Id != sessionId)
            {
                if (selectFirst) SessionError = "这个会话已不存在，请刷新项目列表。";
                else { ActiveSessionId = null; shelvedSession = null; }
            }
            else if ((sessionId ?? (selectFirst ? Sessions.FirstOrDefault()?.Id : null)) is { } selected) await OpenSessionAsync(selected);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (generation == navigationGeneration) SessionError = ex.Message;
        }
        catch (OperationCanceledException) { }
        finally { if (generation == navigationGeneration) { SessionsLoading = false; Notify(); } }
    }

    // Only the acknowledged native creation route may adopt an empty frame that
    // persisted-only history intentionally omits until its first user message.
    public Task OpenCreatedSessionAsync(string projectId, string sessionId)
    {
        if (ActiveProjectId != projectId || lifetime.IsCancellationRequested || string.IsNullOrWhiteSpace(sessionId)) return Task.CompletedTask;
        createdDraft = new(sessionId, projectId, "", DateTimeOffset.UtcNow.ToUnixTimeSeconds(), "idle");
        return OpenProjectAsync(projectId, sessionId);
    }

    // Entry came from the explicitly scoped shelf response. Reading it must not
    // insert it into the main list, search suggestions, or restore its visibility.
    public Task OpenShelvedSessionAsync(string projectId, ShelvedSession session)
    {
        if (ActiveProjectId != projectId || SessionsLoading || lifetime.IsCancellationRequested || string.IsNullOrWhiteSpace(session.Id)) return Task.CompletedTask;
        shelvedSession = new(session.Id, projectId, session.Title, session.Timestamp, "complete");
        return OpenSessionAsync(session.Id);
    }

    // Shelving hides the saved list entry, not the conversation being read.
    // Retain its accepted metadata while refreshing the visible list.
    public Task RefreshAfterShelvingAsync(BrowserSession session)
    {
        if (ActiveProjectId != session.ProjectId || lifetime.IsCancellationRequested) return Task.CompletedTask;
        RecentSessions = RecentSessions.Where(row => row.ProjectId != session.ProjectId || row.Id != session.Id).ToArray();
        if (ActiveSessionId == session.Id) shelvedSession = session;
        return OpenProjectAsync(session.ProjectId, ActiveSessionId, selectFirst: false);
    }

    public Task RefreshAfterTransferAsync(BrowserSession session, bool moved)
    {
        if (ActiveProjectId != session.ProjectId || lifetime.IsCancellationRequested) return Task.CompletedTask;
        if (moved)
        {
            RecentSessions = RecentSessions.Where(row => row.ProjectId != session.ProjectId || row.Id != session.Id).ToArray();
            if (ActiveSessionId == session.Id)
            {
                ActiveSessionId = null; shelvedSession = null; Messages = []; NextBeforeSeq = null;
                ++transcriptGeneration; TranscriptLoading = false; Notify();
            }
        }
        // An older home refresh may still have captured selectFirst=true.
        // Advance navigation while reconciling the source list so that reply
        // cannot select a neighboring conversation after a move.
        return Loading ? OpenProjectAsync(session.ProjectId, ActiveSessionId, selectFirst: false) : RefreshAsync(selectFirst: false);
    }

    public async Task OpenSessionAsync(string id, bool older = false, long? beforeSeq = null)
    {
        if (ActiveProjectId is not { } projectId || (!Sessions.Any(s => s.Id == id) && shelvedSession?.Id != id && explorationSession?.Id != id)) return;
        if (createdDraft is { } draft && draft.Id != id)
        {
            Sessions = Sessions.Where(s => s.Id != draft.Id).ToArray(); createdDraft = null;
        }
        ++explorationOpenGeneration;
        if (explorationSession?.Id == id || createdDraft?.Id == id)
        {
            // Private and newly created empty frames are served by the native
            // conversation API. Persisted-only browser reads reject these frames.
            ++transcriptGeneration; ActiveSessionId = id; Messages = []; NextBeforeSeq = null;
            TranscriptLoading = false; SessionError = null; Notify(); return;
        }
        explorationSession = null;
        if (older && (TranscriptLoading || id != ActiveSessionId || NextBeforeSeq == null)) return;
        if (shelvedSession?.Id != id) shelvedSession = null;
        var generation = ++transcriptGeneration;
        var cursor = older ? NextBeforeSeq : beforeSeq;
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

    public async Task<bool> OpenExplorationAsync(INativeSettingsClient host, string projectId, string explorationId, string sessionId)
    {
        if (ActiveProjectId != projectId || SessionsLoading || lifetime.IsCancellationRequested) return false;
        var ticket = ++explorationOpenGeneration;
        var navigation = navigationGeneration;
        try
        {
            var reply = await host.InvokeAsync("native_conversation_exploration_open",
                new() { ["session_id"] = sessionId, ["exploration_id"] = explorationId }, projectId, lifetime.Token);
            var summary = NativeExplorationOpen.Decode(reply, projectId, explorationId, sessionId);
            if (lifetime.IsCancellationRequested || ticket != explorationOpenGeneration || navigation != navigationGeneration
                || ActiveProjectId != projectId) return false;
            explorationSession = summary.Session(projectId); shelvedSession = null;
            await OpenSessionAsync(sessionId); return true;
        }
        catch (OperationCanceledException) { return false; }
        catch (Exception error)
        {
            if (!lifetime.IsCancellationRequested && ticket == explorationOpenGeneration && navigation == navigationGeneration)
            { SessionError = error.Message; Notify(); }
            return false;
        }
    }

    public IEnumerable<SearchResult> Search(string query)
    {
        bool Matches(string value) => value.Contains(query.Trim(), StringComparison.OrdinalIgnoreCase);
        if (ActiveProjectId == null)
            foreach (var p in HomeProjects.Where(p => Matches(p.Name) || Matches(p.Description) || Matches(p.WorkspaceDirectory)))
                yield return new(p.Name, p.WorkspaceDirectory, p.Id, null);
        foreach (var s in (ActiveProjectId == null ? HomeRecentSessions : Sessions).Where(s => Matches(s.Title)))
            yield return new(s.Title, Projects.FirstOrDefault(p => p.Id == s.ProjectId)?.Name ?? "会话", s.ProjectId, s.Id);
    }

    private void Notify() { if (!lifetime.IsCancellationRequested) Changed?.Invoke(); }
    public void Dispose() { lifetime.Cancel(); lifetime.Dispose(); }
}

public sealed record SearchResult(string Title, string Detail, string ProjectId, string? SessionId);
