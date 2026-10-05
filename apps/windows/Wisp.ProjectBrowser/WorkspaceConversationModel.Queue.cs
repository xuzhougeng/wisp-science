using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeQueueRecovery(string Id, string Text, string Reason);
public sealed partial class WorkspaceConversationModel
{
    private sealed record QueueDraft(string Id, string Epoch, string Text, ComposerAttachment[] Files, NativeReferenceOption[] References, NativeComposerQuote[] Quotes);
    private readonly Dictionary<string, QueueDraft> pendingQueueEnqueues = [];
    private readonly Dictionary<(string Session, string Id), QueueDraft> submittedQueueDrafts = [];
    private readonly Dictionary<(string Session, string Id), string> queueRecoveryReasons = [];
    private readonly HashSet<string> uncertainQueueActions = [];
    private readonly HashSet<(string Session, string Id)> startedQueueDrafts = [];
    private bool QueueEnqueuePending => sessionId != null && pendingQueueEnqueues.ContainsKey(sessionId);
    public NativeQueueItem[] QueuedTurns => Snapshot?.Queue?.Items ?? [];
    public bool QueueUncertain => sessionId != null && (uncertainQueues.Contains(sessionId) || uncertainQueueActions.Contains(sessionId));
    public NativeQueueRecovery[] QueueRecoveries => submittedQueueDrafts.Where(row => row.Key.Session == sessionId && queueRecoveryReasons.ContainsKey(row.Key))
        .Select(row => new NativeQueueRecovery(row.Key.Id, row.Value.Text, queueRecoveryReasons[row.Key])).ToArray();
    public bool CanRecoverQueue => CanAttach && string.IsNullOrEmpty(Draft) && Attachments.Length == 0 && References.Length == 0 && Quotes.Length == 0;
    public void RestoreQueueDraft(string id)
    {
        if (!CanRecoverQueue || sessionId is not { } session || !queueRecoveryReasons.ContainsKey((session, id))
            || !submittedQueueDrafts.TryGetValue((session, id), out var draft)) return;
        Draft = draft.Text; Attachments = draft.Files; References = draft.References; Quotes = draft.Quotes;
        stagedFiles[session] = Attachments; stagedReferences[session] = References; stagedQuotes[session] = Quotes;
        queueRecoveryReasons.Remove((session, id)); submittedQueueDrafts.Remove((session, id)); Notify();
    }
    public void AcknowledgeQueueResult()
    {
        if (sessionId == null || Busy || ConnectionError != null) return;
        uncertainQueueActions.Remove(sessionId); uncertainQueues.Remove(sessionId); RetainUnconfirmedQueueDraft(sessionId);
        OperationError = null; Notify();
    }
    private void RetainUnconfirmedQueueDraft(string session)
    {
        if (pendingQueueEnqueues.Remove(session, out var pendingQueue))
            submittedQueueDrafts[(session, pendingQueue.Id)] = pendingQueue;
    }
    // Consume only the submitted components. Polls and late responses must leave
    // independent text, files and references (including a revisited session) intact.
    private void ConsumeQueuedDraft(string session, QueueDraft sent)
    {
        if (drafts.GetValueOrDefault(session) == sent.Text) drafts[session] = "";
        stagedFiles[session] = stagedFiles.GetValueOrDefault(session, []).Except(sent.Files).ToArray();
        stagedReferences[session] = stagedReferences.GetValueOrDefault(session, []).Except(sent.References).ToArray();
        stagedQuotes[session] = stagedQuotes.GetValueOrDefault(session, []).Except(sent.Quotes).ToArray();
        if (sessionId != session) return;
        if (Draft == sent.Text) Draft = "";
        Attachments = Attachments.Except(sent.Files).ToArray();
        References = References.Except(sent.References).ToArray();
        Quotes = Quotes.Except(sent.Quotes).ToArray();
        stagedFiles[session] = Attachments; stagedReferences[session] = References; stagedQuotes[session] = Quotes;
    }
    private void ReconcileQueue(ConversationSnapshot snapshot)
    {
        if (snapshot.Queue is not { } queue) return;
        var session = snapshot.SessionId;
        if (pendingQueueEnqueues.TryGetValue(session, out var pendingQueue)
            && (queue.Items.Any(item => item.Id == pendingQueue.Id) || queue.Outcomes.Any(item => item.Id == pendingQueue.Id)))
        {
            ConsumeQueuedDraft(session, pendingQueue);
            submittedQueueDrafts[(session, pendingQueue.Id)] = pendingQueue;
            pendingQueueEnqueues.Remove(session); uncertainQueues.Remove(session);
            if (!QueueUncertain) OperationError = null;
        }
        foreach (var entry in submittedQueueDrafts.Where(row => row.Key.Session == session).ToArray())
        {
            var outcome = queue.Outcomes.LastOrDefault(row => row.Id == entry.Key.Id);
            if (outcome?.State == "failed") queueRecoveryReasons[entry.Key] = "排队执行失败，请检查会话后恢复草稿。";
            else if (outcome?.State is "completed" or "cancelled" or "superseded")
            { submittedQueueDrafts.Remove(entry.Key); queueRecoveryReasons.Remove(entry.Key); startedQueueDrafts.Remove(entry.Key); }
            else if (outcome?.State == "started") startedQueueDrafts.Add(entry.Key);
            else if (entry.Value.Epoch != snapshot.Epoch && !startedQueueDrafts.Contains(entry.Key) && !queue.Items.Any(row => row.Id == entry.Key.Id))
                queueRecoveryReasons[entry.Key] = "宿主已重启，排队结果未确认。请核对会话后恢复草稿。";
            else if (entry.Value.Epoch != snapshot.Epoch && startedQueueDrafts.Remove(entry.Key)) submittedQueueDrafts.Remove(entry.Key);
        }
        QueuedFollowUp = queue.Items.FirstOrDefault()?.Message;
    }
    public NativeQueueTarget? QueueTarget(NativeQueueItem item) => Snapshot is { } snapshot
        ? new(snapshot.ProjectId, snapshot.SessionId, item) : null;
    public bool CanQueueAction(NativeQueueTarget target, string kind)
    {
        if (settings == null || Busy || QueueUncertain || UncertainSend || ConnectionError != null || ShowingHistory
            || target.ProjectId != projectId || target.SessionId != sessionId || Snapshot is not { ReadOnly: false, Queue: { } queue }) return false;
        var index = Array.FindIndex(queue.Items, row => row.Id == target.Item.Id && row.Digest == target.Item.Digest && row.State == "queued");
        if (index < 0) return false;
        return kind switch {
            "cut_in" => queue.CanCutIn && Snapshot.Running && !Snapshot.Stopping,
            "replace" => Snapshot.Running && !Snapshot.Stopping,
            "move_up" => queue.Items.Take(index).Any(row => row.State == "queued"),
            "move_down" => queue.Items.Skip(index + 1).Any(row => row.State == "queued"),
            "edit" or "cancel" => true,
            _ => false
        };
    }
    public async Task<bool> QueueActionAsync(NativeQueueTarget target, JsonObject action, CancellationToken token = default)
    {
        if (!CanQueueAction(target, action["kind"]?.GetValue<string>() ?? "")) return false;
        var current = generation; Busy = true; OperationError = null; Notify();
        var success = false;
        try
        {
            await new NativeConversationQueueClient(settings!).ActAsync(target, action, token);
            if (action["kind"]?.GetValue<string>() == "edit" && submittedQueueDrafts.TryGetValue((target.SessionId, target.Item.Id), out var original))
                submittedQueueDrafts[(target.SessionId, target.Item.Id)] = original with { Text = HistoryDraft(action["message"]!.GetValue<string>()), Quotes = [] };
            success = true;
        }
        catch (Exception ex)
        {
            uncertainQueueActions.Add(target.SessionId);
            if (current == generation) OperationError = "队列操作结果未确认，请重新读取后核对；不会自动重试。" + ex.Message;
        }
        finally { if (current == generation) { Busy = false; Notify(); } }
        if (current == generation && !token.IsCancellationRequested) await RefreshAsync(token);
        return current == generation && success;
    }
}
