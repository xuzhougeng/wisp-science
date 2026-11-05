using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeErrorTarget(string ProjectId, string SessionId, string Epoch, string? RequestId,
    string? Failure, string RowKey, string Message);

public sealed partial class WorkspaceConversationModel
{
    private const string ResumeFailurePrefix = "Resume was not confirmed. Reload the conversation before continuing; it will not be retried automatically.\n";
    public string? ResumeFailureDetail => OperationError?.StartsWith(ResumeFailurePrefix, StringComparison.Ordinal) == true
        ? OperationError[ResumeFailurePrefix.Length..] : null;
    private readonly HashSet<(string Project, string Session, string Epoch, string Row, string Message)> resumedErrors = [];
    private static (string, string, string, string, string) ErrorIdentity(NativeErrorTarget target)
        => (target.ProjectId, target.SessionId, target.Epoch, target.RowKey, target.Message);

    public NativeErrorTarget? ErrorTarget(ConversationSnapshot page, int row)
    {
        if (Snapshot is not { } current || page.ProjectId != current.ProjectId || page.SessionId != current.SessionId
            || page.Epoch != current.Epoch || row < 0 || row >= page.Items.Length
            || NativeErrorPresentation.Message(page.Items[row]) is not { } message) return null;
        var key = page.Items[row].Kind == "native_turn_error" ? "failure:" + current.RequestId : NativeTranscriptRows.Keys(page)[row];
        return new(page.ProjectId, page.SessionId, current.Epoch, current.RequestId, current.Error, key, message);
    }

    public bool ErrorWasResumed(NativeErrorTarget? target) => target != null && resumedErrors.Contains(ErrorIdentity(target));
    public bool ShowsErrorResume(NativeErrorTarget? target) => target != null && !target.Message.StartsWith(NativeErrorPresentation.AcpPrefix, StringComparison.Ordinal)
        && target.Failure?.StartsWith(NativeErrorPresentation.AcpPrefix, StringComparison.Ordinal) != true
        && Snapshot is { ReadOnly: false, AcpAgentId: null } snapshot && !snapshot.ModelId.StartsWith("acp:", StringComparison.Ordinal)
        && ComposerAgentId == null;
    public bool CanResumeError(NativeErrorTarget? target) => ShowsErrorResume(target) && target != null && active && ContextReady
        && !Loading && !Busy && !HistoryBusy && !UncertainSend && !QueueUncertain && !QueueEnqueuePending
        && !CompactionBlocksInput && !Effort.Busy && !Options.Busy && ContextRecovery?.Starting != true && ConnectionError == null
        && !ErrorWasResumed(target) && Guid.TryParse(target.RequestId, out _)
        && Snapshot is { Running: false, Stopping: false } snapshot && target.ProjectId == projectId && target.SessionId == sessionId
        && target.Epoch == snapshot.Epoch && target.RequestId == snapshot.RequestId && target.Failure == snapshot.Error;

    public async Task ResumeErrorAsync(NativeErrorTarget target, CancellationToken token = default)
    {
        if (token.IsCancellationRequested || !CanResumeError(target)) return;
        ContextRecovery = null; IdleCompactionPrompt = null;
        if (ShowingHistory) Latest();
        await ResumeRequestAsync(target.ProjectId, target.SessionId, target.RequestId!, generation, token,
            () => resumedErrors.Add(ErrorIdentity(target)));
    }

    // Both the explicit error action and successful compaction use the same
    // empty-message request and uncertainty ledger, without touching composer data.
    private async Task ResumeRequestAsync(string project, string session, string after, int owner, CancellationToken token, Action? accepted = null)
    {
        var request = Guid.NewGuid(); var key = (project, session);
        uncertainRecoveryResumes[key] = request; Busy = true; OperationError = null; Notify();
        try
        {
            await client.ResumeAsync(project, session, request, after, token);
            uncertainRecoveryResumes.Remove(key); accepted?.Invoke();
        }
        catch (Exception ex)
        {
            if (owner == generation)
            { UncertainSend = true; OperationError = ResumeFailurePrefix + ex.Message; }
        }
        finally { if (owner == generation) { Busy = false; Notify(); } }
        if (owner == generation && !token.IsCancellationRequested) await RefreshAsync(token);
    }
}
