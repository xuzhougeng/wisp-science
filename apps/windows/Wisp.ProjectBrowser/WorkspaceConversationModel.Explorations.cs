using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    public NativeHistoryTarget? ExplorationTarget(ConversationSnapshot page, int row)
    {
        if (AssistantMode || Snapshot is not { AcpAgentId: null, HistoryState.CanBranch: true } current
            || current.Exploration?.Summary != null || HistoryTarget(page, row) is not { Role: "assistant" } target
            || (current.Running || current.Stopping) && target.Turn.UserIndex == current.HistoryState.Turns.Length - 1)
            return null;
        return target;
    }
    public NativeHistoryTarget? CurrentExplorationRoundTarget => Snapshot is
        { Exploration: { SourceFrozen: true, RoundUserIndex: { } index } round, HistoryState: { } history } page
        && round.RoundSourceFrameId == page.SessionId && index >= 0 && index < history.Turns.Length
        ? new(page.ProjectId, page.SessionId, history.Turns[index], history.Revision, "assistant", "") : null;

    public bool CanStartExploration(NativeHistoryTarget target)
    {
        if (settings == null || !active || AssistantMode || Busy || HistoryBusy || UncertainSend || HistoryUncertain || ConnectionError != null
            || target.ProjectId != projectId || target.SessionId != sessionId || target.Role != "assistant"
            || Snapshot is not { Running: false, Stopping: false, AcpAgentId: null, HistoryState.CanBranch: true } page
            || page.Exploration?.Summary != null || page.ReadOnly && page.Exploration?.SourceFrozen != true
            || target.Turn.UserIndex < 0 || target.Turn.UserIndex >= page.HistoryState.Turns.Length
            || page.HistoryState.Turns[target.Turn.UserIndex] != target.Turn) return false;
        return page.Exploration?.RoundSourceFrameId is not { } source
            ? page.Exploration?.SourceFrozen != true
            : source == sessionId && page.Exploration.RoundUserIndex == target.Turn.UserIndex;
    }
}
