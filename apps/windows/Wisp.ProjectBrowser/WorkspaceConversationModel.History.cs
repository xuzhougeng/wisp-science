using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    private readonly HashSet<string> uncertainHistory = [];
    public bool HistoryBusy { get; private set; }
    public bool HistoryUncertain => sessionId != null && uncertainHistory.Contains(sessionId);
    public void AcknowledgeHistoryResult()
    {
        if (sessionId == null || ConnectionError != null || Busy) return;
        uncertainHistory.Remove(sessionId); OperationError = null; Notify();
    }
    public NativeHistoryTarget? HistoryTarget(ConversationSnapshot page, int row) => HistoryTurnAt(page, row) is { } at
        ? new(page.ProjectId, page.SessionId, at.Turn, page.HistoryState!.Revision, page.Items[row].Role,
            HistoryDraft(page.Items[at.UserRow].Text))
        : null;
    /// <summary>The turn a row belongs to, without building its rewind draft.</summary>
    public NativeTurnIdentity? HistoryTurn(ConversationSnapshot page, int row) => HistoryTurnAt(page, row)?.Turn;
    private (NativeTurnIdentity Turn, int UserRow)? HistoryTurnAt(ConversationSnapshot page, int row)
    {
        if (page.SessionId != sessionId || page.ProjectId != projectId || row < 0 || row >= page.Items.Length
            || page.Items[row].Role is not ("user" or "assistant") || page.HistoryState == null) return null;
        var user = (page.UserOffset ?? 0) - 1; var userRow = -1;
        for (var n = 0; n <= row; n++) if (page.Items[n].Role == "user") { user++; userRow = n; }
        if (userRow < 0 || user < 0 || user >= page.HistoryState.Turns.Length) return null;
        return (page.HistoryState.Turns[user], userRow);
    }
    public static string HistoryDraft(string text)
    {
        string[] markers = ["\n\nUploaded files: ", "\n\nAttached artifacts: ", "\n\nAttached sessions: ", "\n\nProject context: ",
            "\n\nSelected skills: ", "\n\nSelected workflows: ", "\n\nTarget environments: ", "\n\nTarget runtimes: ",
            "\n\nAI source-edit instruction: ", "\n\nFeedback context: "];
        var end = markers.Select(marker => text.IndexOf(marker, StringComparison.Ordinal)).Where(index => index >= 0).DefaultIfEmpty(text.Length).Min();
        return end == text.Length ? text : text[..end].Trim();
    }
    public bool CanHistoryAction(NativeHistoryTarget target, string kind)
    {
        if (settings == null || !active || Busy || HistoryBusy || UncertainSend || HistoryUncertain || ConnectionError != null
            || target.ProjectId != projectId || target.SessionId != sessionId || Snapshot is not { ReadOnly: false, HistoryState: { } state }
            || target.Turn.UserIndex < 0 || target.Turn.UserIndex >= state.Turns.Length || state.Turns[target.Turn.UserIndex] != target.Turn) return false;
        var latest = target.Turn.UserIndex == state.Turns.Length - 1;
        var running = Snapshot.Running || Snapshot.Stopping;
        return kind switch {
            "branch" => state.CanBranch && (!running || !latest),
            "propose_memory" or "confirm_memory" => target.Role == "assistant" && (!running || !latest),
            "review" => target.Role == "assistant" && !running && !state.Reviewing,
            "rewind" => target.Role == "user" && !running && !state.Reviewing && Snapshot.AcpAgentId == null,
            "undo_preview" or "undo" => target.Role == "assistant" && latest && !running && !state.Reviewing && Snapshot.AcpAgentId == null,
            _ => false
        };
    }
    // Rebind the revision when opening the confirmation, never when confirming it.
    public NativeHistoryTarget CurrentHistoryTarget(NativeHistoryTarget target) =>
        target with { Revision = Snapshot?.HistoryState?.Revision ?? target.Revision };

    public async Task<(bool Success, JsonNode? Result)> HistoryActionAsync(NativeHistoryTarget target, JsonObject action, CancellationToken token = default)
    {
        var kind = action["kind"]?.GetValue<string>() ?? "";
        if (!CanHistoryAction(target, kind)) return (false, null);
        var current = generation; var draftAtStart = Draft;
        var blocksComposer = kind != "propose_memory";
        HistoryBusy = true; if (blocksComposer) Busy = true;
        OperationError = null; Notify();
        var success = false; JsonNode? result = null;
        try
        {
            result = await new NativeConversationHistoryClient(settings!).ActAsync(target, action, token);
            if (current != generation) return (false, null);
            if (kind is "rewind" or "undo")
            {
                History = null; ShowingHistory = false;
                // A draft typed while the request was pending belongs to the user.
                if (Draft == draftAtStart && string.IsNullOrEmpty(draftAtStart)) Draft = target.Draft;
            }
            if (kind == "branch" && result?.GetValue<string>() is { Length: > 0 } id)
                drafts[id] = action["checkpoint"]?.GetValue<string>() == "before_user" ? target.Draft : "";
            success = true;
        }
        catch (Exception ex)
        {
            // Even a failed response may follow a committed mutation. Reads do
            // not acknowledge it; only an explicit user check permits another attempt.
            if (kind is not ("undo_preview" or "propose_memory")) uncertainHistory.Add(target.SessionId);
            if (current == generation) OperationError = "历史操作未确认，请核对会话后再继续。" + ex.Message;
        }
        finally { if (current == generation) { HistoryBusy = false; if (blocksComposer) Busy = false; Notify(); } }
        if (current == generation && !token.IsCancellationRequested) await RefreshAsync(token);
        return current == generation ? (success, result) : (false, null);
    }

    public static NativeTurnMemoryProposal DecodeMemory(JsonNode node, NativeHistoryTarget target)
    {
        var result = node.Deserialize<NativeTurnMemoryProposal>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing memory proposal");
        if (result.SessionId != target.SessionId || result.TurnIndex != target.Turn.UserIndex || result.GlobalMemories == null
            || result.Scope is not ("project" or "global") || string.IsNullOrWhiteSpace(result.Content))
            throw new InvalidDataException("Memory proposal identity mismatch");
        return result;
    }
}
