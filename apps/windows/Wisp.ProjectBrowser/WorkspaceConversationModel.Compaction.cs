using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeCompactionState(Guid RequestId, string Epoch, bool Semantic, string Instruction,
    bool Submitting = false, bool Acknowledged = false, string? Error = null, bool Visible = true, bool Started = true)
{
    public bool Busy => Started && (Submitting || Error == null);
    public bool Uncertain => Started && !Acknowledged && Error != null;
}

public sealed partial class WorkspaceConversationModel
{
    private readonly Dictionary<(string Project, string Session), NativeCompactionState> compactions = [];
    public NativeCompactionState? Compaction => active && projectId != null && sessionId != null
        ? compactions.GetValueOrDefault((projectId, sessionId)) : null;
    private bool CompactionBlocksInput => Compaction is { } state && (state.Busy || state.Uncertain);
    public bool CanRunCompaction => active && ContextReady && !Busy && !Loading && !ShowingHistory
        && ComposerAgentId == null && Snapshot is { Running: false, ReadOnly: false } && ConnectionError == null
        && Compaction is not { Busy: true } && !UncertainSend && !QueueEnqueuePending && !QueueUncertain;
    public bool CanOpenCompaction => CanAttach && CanRunCompaction && !Effort.Busy && !Options.Busy;

    public bool OpenCompaction(bool semantic = false, string instruction = "")
    {
        if (!CanOpenCompaction || projectId == null || sessionId == null || Snapshot == null) return false;
        DismissIdleCompaction();
        recoveryCompactionOwner = null; recoveryResumeReady = null;
        compactions[(projectId, sessionId)] = new(Guid.Empty, Snapshot.Epoch, semantic, instruction, Started: false);
        Notify(); return true;
    }

    public void UpdateCompactionDraft(bool semantic, string instruction)
    {
        if (Compaction is not { Busy: false, Uncertain: false } state || state.Semantic == semantic && state.Instruction == instruction) return;
        compactions[(projectId!, sessionId!)] = state with { Semantic = semantic, Instruction = instruction }; Notify();
    }

    public bool HandleCompactionCommand(string command)
    {
        if (!NativeComposerCommands.TryCompaction(command, out var instruction)) return false;
        if (!CanCompose) return true;
        if (ComposerAgentId != null) OperationError = "ACP conversations cannot be compacted from this desktop view.";
        else if (sessionId == null) OperationError = "Open a conversation before compacting its context.";
        else if (!CanOpenCompaction) { OperationError = "Wait for the conversation to finish before compacting its context."; Notify(); return true; }
        else OperationError = null;
        // WebView consumes the command, but keeps any attached composer context.
        Draft = "";
        if (OperationError == null) OpenCompaction(instruction.Length > 0, instruction);
        else Notify();
        return true;
    }

    public void ShowCompaction()
    {
        if (Compaction is { } state) { compactions[(projectId!, sessionId!)] = state with { Visible = true }; Notify(); }
    }
    public void DismissCompaction()
    {
        if (Compaction is not { Busy: false } state) return;
        recoveryCompactionOwner = null; recoveryResumeReady = null;
        if (state.Uncertain) compactions[(projectId!, sessionId!)] = state with { Visible = false };
        else compactions.Remove((projectId!, sessionId!));
        Notify();
    }

    // A separate command leaves the user's draft, attachments and quote context intact.
    // Unconfirmed retries reuse the request identity and payload for host deduplication.
    public async Task StartCompactionAsync(bool semantic = true, string instruction = "", CancellationToken token = default)
    {
        if (token.IsCancellationRequested || !CanRunCompaction
            || Snapshot is not { Running: false, ReadOnly: false } snapshot || projectId is not { } project
            || sessionId is not { } session) return;
        var key = (project, session); var owner = generation;
        var state = Compaction is { Uncertain: true } uncertain ? uncertain
            : new NativeCompactionState(Guid.NewGuid(), snapshot.Epoch, semantic, instruction.Trim());
        state = state with { Submitting = true, Error = null, Visible = true };
        compactions[key] = state; Notify();
        try
        {
            var command = state.Semantic ? "/compact --semantic" : "/compact";
            if (state.Instruction.Length > 0) command += " " + state.Instruction;
            await client.SendAsync(project, session, state.RequestId, command, token);
            if (compactions.GetValueOrDefault(key)?.RequestId == state.RequestId)
                compactions[key] = state with { Submitting = false, Acknowledged = true };
        }
        catch (Exception ex)
        {
            if (compactions.GetValueOrDefault(key)?.RequestId == state.RequestId)
                compactions[key] = state with { Submitting = false, Error = ex.Message };
        }
        if (owner != generation) return;
        Notify();
        if (!token.IsCancellationRequested) await RefreshAsync(token);
    }

    private void ReconcileCompaction(ConversationSnapshot snapshot)
    {
        var key = (snapshot.ProjectId, snapshot.SessionId);
        if (!compactions.TryGetValue(key, out var state) || !state.Started || state.Submitting) return;
        if (snapshot.Epoch != state.Epoch)
        {
            // A restarted host has lost its request deduplication ledger. Never replay it there.
            compactions[key] = state with { Acknowledged = true, Error = "The connection restarted. Check the conversation before starting compaction again." };
            return;
        }
        if (snapshot.RequestId != state.RequestId.ToString())
        {
            if (state.Acknowledged && recoveryCompactionOwner == (generation, snapshot.ProjectId, snapshot.SessionId, snapshot.Epoch))
            {
                recoveryCompactionOwner = null;
                compactions[key] = state with { Error = "Another request changed the conversation. Automatic continuation was cancelled." };
            }
            return;
        }
        if (snapshot.Running) compactions[key] = state with { Acknowledged = true, Error = null };
        else if (snapshot.Error != null) compactions[key] = state with { Acknowledged = true, Error = snapshot.Error };
        else { compactions.Remove(key); QueueRecoveryResume(snapshot, state); }
    }
}
