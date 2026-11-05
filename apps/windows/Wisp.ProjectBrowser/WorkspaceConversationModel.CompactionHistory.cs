using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    private readonly Dictionary<NativeCompactionTarget, NativeCompactionUndoOutcome> compactionUndos = [];
    public NativeCompactionUndoOutcome? CompactionUndoOutcome(NativeCompactionTarget? target) => target == null ? null : compactionUndos.GetValueOrDefault(target);
    public NativeCompactionTarget? CompactionTarget(ConversationSnapshot page, NativeCompactionRecord row) => row.Epoch is { } epoch
        ? new(page.ProjectId, page.SessionId, page.Epoch, epoch) : null;
    public bool CanUndoCompaction(NativeCompactionTarget? target)
    {
        if (target == null || settings == null || !CanRunCompaction || HistoryBusy || HistoryUncertain || CompactionBlocksInput
            || Snapshot is not { Stopping: false } snapshot || snapshot.HistoryState?.Reviewing == true || snapshot.Epoch != target.HostEpoch
            || projectId != target.ProjectId || sessionId != target.SessionId || CompactionUndoOutcome(target) != null) return false;
        var rows = snapshot.Items.Select(NativeCompactionRecord.Parse).Where(row => row?.Epoch == target.Epoch).ToArray();
        return rows.Length == 1 && rows[0] is { CanUndo: true, Undone: false, AutomaticContinue: false };
    }
    public NativeHistoryTarget? CompactionRewindTarget(ConversationSnapshot page, NativeCompactionRecord row)
    {
        if (!ContextReady || CompactionBlocksInput || row.Undone || row.AutomaticContinue || page.Epoch != Snapshot?.Epoch || row.KeptFromUserIndex is not { } kept
            || NativeCompactionRecord.RewindRow(page, kept) is not { } index) return null;
        if (CompactionUndoOutcome(CompactionTarget(page, row)) != null || page.HistoryState is not { } history
            || kept >= history.Turns.Length || history.Turns[kept].UserIndex != kept) return null;
        var target = new NativeHistoryTarget(page.ProjectId, page.SessionId, history.Turns[kept], history.Revision,
            "user", HistoryDraft(page.Items[index].Text));
        return CanHistoryAction(target, "rewind") ? CurrentHistoryTarget(target) : null;
    }
    public async Task UndoCompactionAsync(NativeCompactionTarget target, CancellationToken token = default)
    {
        if (token.IsCancellationRequested || !CanUndoCompaction(target)) return;
        // Keep the in-flight guard across navigation away and back to this session.
        compactionUndos[target] = new();
        var owner = generation; Busy = true; HistoryBusy = true; Notify();
        try
        {
            var reply = await settings!.InvokeAsync("native_conversation_context_undo", new JsonObject {
                ["session_id"] = target.SessionId, ["head_epoch"] = target.Epoch
            }, target.ProjectId, token);
            if (reply?["project_id"]?.GetValue<string>() != target.ProjectId || reply?["session_id"]?.GetValue<string>() != target.SessionId
                || reply?["undone_epoch"]?.GetValue<ulong>() != target.Epoch) throw new InvalidDataException("Compaction undo response identity mismatch");
            compactionUndos[target] = new(Confirmed: true);
            if (owner == generation) ContextDetails.Invalidate();
        }
        catch (Exception ex) { compactionUndos[target] = new(Error: ex.Message); }
        finally
        {
            if (owner == generation)
            {
                if (!token.IsCancellationRequested) await RefreshAsync(token);
                if (owner == generation) { Busy = false; HistoryBusy = false; Notify(); }
            }
        }
    }
    public async Task ReloadCompactionUndoAsync(NativeCompactionTarget target, CancellationToken token = default)
    {
        if (token.IsCancellationRequested || Busy || target.ProjectId != projectId || target.SessionId != sessionId || target.HostEpoch != Snapshot?.Epoch) return;
        var owner = generation; var before = Snapshot.Sequence;
        await RefreshAsync(token);
        if (!token.IsCancellationRequested && owner == generation && ConnectionError == null && Snapshot is { } snapshot && snapshot.Sequence != before
            && compactionUndos.GetValueOrDefault(target)?.Error != null)
        {
            // Only this explicit successful read permits another user-triggered
            // attempt. The same expected epoch still protects the host mutation.
            compactionUndos.Remove(target); Notify();
        }
    }
}
