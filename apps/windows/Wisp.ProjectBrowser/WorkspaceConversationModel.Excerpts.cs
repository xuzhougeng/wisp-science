using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    private NativeHistoryTarget? excerptStart;
    private readonly HashSet<(string Project, string Session)> excerptPending = [];
    private readonly Dictionary<(string Project, string Session), string> excerptCreationFailures = [];
    private string? excerptReadFailure;
    private readonly HashSet<string> truncatedExcerpts = [];
    public NativeHistoryTarget? ExcerptStart => excerptStart is { } start && ValidExcerptTurn(start) ? start : null;
    public bool ExcerptBusy => projectId != null && sessionId != null && excerptPending.Contains((projectId, sessionId));
    public bool ExcerptUncertain => projectId != null && sessionId != null && excerptCreationFailures.ContainsKey((projectId, sessionId));
    public string? ExcerptFailure => projectId != null && sessionId != null
        ? excerptCreationFailures.GetValueOrDefault((projectId, sessionId)) ?? excerptReadFailure : null;
    public bool ExcerptTruncated => sessionId != null && truncatedExcerpts.Contains(sessionId)
        && Quotes.Any(quote => quote.Role == "excerpt");

    private bool ValidExcerptTurn(NativeHistoryTarget target) => active && target.Role == "user"
        && target.ProjectId == projectId && target.SessionId == sessionId
        && Snapshot is { AcpAgentId: null, HistoryState: { } history }
        && target.Turn.UserIndex >= 0 && target.Turn.UserIndex < history.Turns.Length
        && history.Turns[target.Turn.UserIndex] == target.Turn
        && (!(Snapshot.Running || Snapshot.Stopping) || target.Turn.UserIndex < history.Turns.Length - 1);

    public bool CanExcerpt(NativeHistoryTarget target) => settings != null && ValidExcerptTurn(target)
        && !Loading && !Busy && !HistoryBusy && !UncertainSend && !HistoryUncertain
        && !ExcerptBusy && !ExcerptUncertain && ConnectionError == null;
    public bool CancelExcerpt()
    {
        if (excerptStart == null) return false;
        excerptStart = null; Notify(); return true;
    }
    public void AcknowledgeExcerptCreation()
    {
        if (projectId == null || sessionId == null || ExcerptBusy || Busy || ConnectionError != null) return;
        excerptCreationFailures.Remove((projectId, sessionId)); excerptReadFailure = null; Notify();
    }

    /// Two explicit clicks; only a confirmed read can create a destination.
    /// No send, source mutation, or automatic retry occurs in this workflow.
    public async Task<string?> SelectExcerptAsync(NativeHistoryTarget target, NativeLocalization locale, CancellationToken token = default)
    {
        if (token.IsCancellationRequested || !CanExcerpt(target)) return null;
        if (ExcerptStart is not { } start) { excerptStart = target; excerptReadFailure = null; Notify(); return null; }
        var first = start.Turn.UserIndex <= target.Turn.UserIndex ? start : target;
        var last = start.Turn.UserIndex <= target.Turn.UserIndex ? target : start;
        var current = generation; var scope = (target.ProjectId, target.SessionId);
        excerptStart = null; excerptReadFailure = null; excerptPending.Add(scope); Busy = true; Notify();
        var creating = false;
        try
        {
            var value = await new NativeConversationExcerptClient(settings!).ReadAsync(first, last, token);
            if (current != generation || token.IsCancellationRequested || !ValidExcerptTurn(first) || !ValidExcerptTurn(last)) return null;
            var quote = NativeComposerQuote.FromExcerpt(value, locale);
            creating = true;
            var id = await client.CreateAsync(target.ProjectId, token);
            if (string.IsNullOrWhiteSpace(id) || id == target.SessionId || stagedQuotes.ContainsKey(id) || drafts.ContainsKey(id))
                throw new InvalidDataException("New conversation identity was not confirmed");
            // Keep the acknowledged destination's quote even if navigation won
            // the race. Opening it later must not require another creation.
            drafts[id] = ""; stagedReferences[id] = []; stagedFiles[id] = []; stagedQuotes[id] = [quote];
            if (value.Truncated) truncatedExcerpts.Add(id);
            return current == generation && !token.IsCancellationRequested ? id : null;
        }
        catch (Exception ex)
        {
            if (creating) excerptCreationFailures[scope] = ex.Message;
            else if (current == generation) excerptReadFailure = ex.Message;
            return null;
        }
        finally
        {
            excerptPending.Remove(scope);
            if (current == generation) { Busy = false; Notify(); }
        }
    }
}
