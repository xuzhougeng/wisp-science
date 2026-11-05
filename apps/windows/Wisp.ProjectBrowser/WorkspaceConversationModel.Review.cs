using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    public bool JumpToReviewMessage(ConversationSnapshot source, int reportRow, int findingIndex)
    {
        var current = ShowingHistory ? History : Snapshot;
        if (!active || current == null || projectId != source.ProjectId || sessionId != source.SessionId
            || current.Epoch != source.Epoch || current.UserOffset != source.UserOffset || current.NextBeforeSeq != source.NextBeforeSeq
            || reportRow < 0 || reportRow >= current.Items.Length || reportRow >= source.Items.Length) return false;
        // Polling may replace the source turn or report while an old button is
        // queued. Accept sequence-only refreshes, not a changed indexing prefix.
        for (var i = 0; i <= reportRow; i++)
        {
            var old = source.Items[i]; var now = current.Items[i];
            if (old.Role != now.Role || old.Text != now.Text || old.ToolName != now.ToolName || old.Kind != now.Kind) return false;
        }
        if (NativeReviewPresentation.JumpIndex(current, reportRow, findingIndex) is not { } index) return false;
        RevealedExcerpt = null; ScrollTarget = index; ScrollRevision++; Notify(); return true;
    }
}
