using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspacePanelModel
{
    private Task<NativeContextActivity>? pendingActivityRead;
    private int pendingActivityRevision, activityRevision;
    public bool RunPolling { get; private set; }
    public DateTimeOffset? RunReadStartedAt { get; private set; }
    public string? RunRefreshError { get; private set; }
    public bool CanPollRuns => !closed && sessionId != null && activity != null;
    public bool RunReadDelayed(DateTimeOffset now) => RunReadStartedAt is { } start && now - start >= TimeSpan.FromSeconds(10);

    // Explicit reads and the timer share one transport request. A read started
    // before a mutation must drain before a fresh post-mutation read can start.
    private async Task<NativeContextActivity> ReadActivityAsync(CancellationToken token)
    {
        while (pendingActivityRead is { } older && pendingActivityRevision != activityRevision)
        {
            try { await older.WaitAsync(token); }
            catch when (!token.IsCancellationRequested) { }
            if (ReferenceEquals(older, pendingActivityRead)) pendingActivityRead = null;
        }
        token.ThrowIfCancellationRequested();
        if (pendingActivityRead == null)
        {
            pendingActivityRevision = activityRevision;
            RunReadStartedAt = DateTimeOffset.UtcNow;
            try { pendingActivityRead = activity!.ReadAsync(projectId, SessionId, token); }
            catch { RunReadStartedAt = null; throw; }
        }
        var pending = pendingActivityRead;
        try { return await pending.WaitAsync(token); }
        finally
        {
            if (pending.IsCompleted && ReferenceEquals(pending, pendingActivityRead))
            { pendingActivityRead = null; RunReadStartedAt = null; }
        }
    }

    /// <summary>Read-only refresh independent of view navigation and mutation errors.</summary>
    public async Task<bool> PollRunsAsync(CancellationToken token = default)
    {
        if (!CanPollRuns || RunPolling || Loading || ActivityBusy || ContextBusy || token.IsCancellationRequested) return false;
        RunPolling = true;
        var epoch = generation; var revision = activityRevision;
        try
        {
            var result = await ReadActivityAsync(token);
            if (closed || epoch != generation || revision != activityRevision || token.IsCancellationRequested) return false;
            var previous = SessionRuns;
            var changed = RunRefreshError != null || Activity?.ReadOnly != result.ReadOnly
                || Activity?.RunReviewSupported != result.RunReviewSupported;
            Activity = result; RunRefreshError = null;
            return changed || !previous.RenderEquivalent(SessionRuns);
        }
        catch (OperationCanceledException) { return false; }
        catch (Exception ex)
        {
            if (closed || epoch != generation || revision != activityRevision) return false;
            var changed = RunRefreshError != ex.Message; RunRefreshError = ex.Message; return changed;
        }
        finally { RunPolling = false; }
    }
}
