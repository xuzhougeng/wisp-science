using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeContextRecovery(string ProjectId, string SessionId, string Epoch, string RequestId, string Failure,
    bool Starting = false, bool Uncertain = false, string? Error = null);
public sealed record NativeContextRecoveryLaunch(NativeContextRecovery Source, string Prompt);

public static class NativeContextErrors
{
    public static bool IsContextLimit(string? error) => NativeErrorPresentation.HintKey(error) == "err.hint.context";
}

public sealed partial class WorkspaceConversationModel
{
    public NativeContextRecovery? ContextRecovery { get; private set; }
    private readonly HashSet<NativeContextRecovery> observedContextFailures = [];
    private (int Generation, string Project, string Session, string Epoch)? recoveryCompactionOwner;
    private (int Generation, string Project, string Session, string Epoch, Guid After)? recoveryResumeReady;
    private readonly Dictionary<(string Project, string Session), Guid> uncertainRecoveryResumes = [];
    public bool CanShowContextRecovery => ContextRecovery is { } state && RecoverySourceMatches(state) && !ShowingHistory;
    public bool CanRecoverContext => ContextRecovery is { Starting: false } && CanShowContextRecovery && CanOpenCompaction;
    public bool RecoverySourceMatches(NativeContextRecovery source) => active && ContextReady && projectId == source.ProjectId
        && sessionId == source.SessionId && Snapshot is { ReadOnly: false, Running: false, Stopping: false, AcpAgentId: null } snapshot
        && !snapshot.ModelId.StartsWith("acp:", StringComparison.Ordinal) && snapshot.Epoch == source.Epoch
        && snapshot.RequestId == source.RequestId && snapshot.Error == source.Failure;

    private void ObserveContextRecovery(ConversationSnapshot? previous, ConversationSnapshot value)
    {
        if (ContextRecovery is { Starting: false } current && !RecoverySourceMatches(current)) ContextRecovery = null;
        if (uncertainRecoveryResumes.TryGetValue((value.ProjectId, value.SessionId), out var resume) && value.RequestId == resume.ToString())
        { uncertainRecoveryResumes.Remove((value.ProjectId, value.SessionId)); UncertainSend = false; OperationError = null; }
        if (!NativeContextErrors.IsContextLimit(value.Error) || value.RequestId is not { Length: > 0 } request) return;
        var failure = new NativeContextRecovery(value.ProjectId, value.SessionId, value.Epoch, request, value.Error!);
        if (!observedContextFailures.Add(failure)) return;
        // A saved error loaded on entry is not a new provider event. Polling a
        // live request's transition is; repeated snapshots cannot reopen it.
        if (previous == null || !RecoverySourceMatches(failure)) return;
        IdleCompactionPrompt = null;
        ContextRecovery = failure;
    }

    public void DismissContextRecovery()
    {
        if (ContextRecovery?.Starting == true) return;
        ContextRecovery = null; Notify();
    }
    public bool CompactAndContinue()
    {
        if (!CanRecoverContext || ContextRecovery is not { } source || !OpenCompaction(semantic: true)) return false;
        recoveryCompactionOwner = (generation, source.ProjectId, source.SessionId, source.Epoch);
        ContextRecovery = null; Notify(); return true;
    }
    private void QueueRecoveryResume(ConversationSnapshot snapshot, NativeCompactionState state)
    {
        if (recoveryCompactionOwner == (generation, snapshot.ProjectId, snapshot.SessionId, snapshot.Epoch))
            recoveryResumeReady = (generation, snapshot.ProjectId, snapshot.SessionId, snapshot.Epoch, state.RequestId);
        recoveryCompactionOwner = null;
    }
    private async Task ResumeRecoveredTurnAsync(CancellationToken token)
    {
        if (recoveryResumeReady is not { } ready) return;
        recoveryResumeReady = null;
        if (token.IsCancellationRequested || ready.Generation != generation || !active || !CanOpenCompaction
            || Snapshot?.Epoch != ready.Epoch || Snapshot.RequestId != ready.After.ToString() || Snapshot.Error != null) return;
        await ResumeRequestAsync(ready.Project, ready.Session, ready.After.ToString(), ready.Generation, token);
    }

    public async Task StartContextRecoverySessionAsync(string prompt, Func<NativeContextRecoveryLaunch, Task> launch)
    {
        if (!CanRecoverContext || ContextRecovery is not { Uncertain: false } source || string.IsNullOrWhiteSpace(prompt)) return;
        var owner = generation; ContextRecovery = source with { Starting = true }; Notify();
        try
        {
            await launch(new(source, prompt));
            if (owner == generation) ContextRecovery = null;
        }
        catch (Exception ex)
        { if (owner == generation) ContextRecovery = source with { Uncertain = true, Error = ex.Message }; }
        finally { if (owner == generation) Notify(); }
    }
    public async Task<string> PrepareContextRecoverySessionAsync(NativeContextRecoveryLaunch launch)
    {
        if (!RecoverySourceMatches(launch.Source) || !CanOpenCompaction || string.IsNullOrWhiteSpace(launch.Prompt))
            throw new InvalidOperationException("The source conversation changed. Recovery was not started.");
        var id = await CreateAsync(launch.Source.ProjectId);
        if (id == null) throw new IOException(OperationError ?? "New conversation creation was not confirmed.");
        // Keep all source composer data. The destination carries only the
        // localized continuation instruction and the canonical session reference.
        drafts[id] = launch.Prompt;
        stagedReferences[id] = [new(new("session", launch.Source.SessionId), launch.Source.SessionId, "")];
        return id;
    }
}
