using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    public bool CanReceiveSessionReference => CanAttach && Snapshot?.ComposerReferences == true || CanPrepareProjectDraft;

    public int ReferenceSidebarSessions(NativeSessionDrag drag, NativeSessionReferenceScope scope, int revision)
    {
        if (revision != generation || !CanReceiveSessionReference || !drag.Valid
            || drag.ProjectId != projectId || scope.ProjectId != projectId) return 0;
        var added = 0;
        // Preserve sidebar catalog order, including pinned members of a folder.
        // A drag never supplies its own titles, membership or reference payload.
        foreach (var source in scope.Sessions.Where(source => source.ProjectId == projectId && source.Id != sessionId
            && (drag.Kind == "session" ? source.Id == drag.Id : source.FolderId == drag.Id)))
        {
            var reference = new NativeReferenceOption(new("session", Id: source.Id), source.Title, scope.ProjectName);
            if (References.Any(existing => existing.Reference.Key == reference.Reference.Key)) continue;
            if (AddReference(reference)) added++;
        }
        return added;
    }
}
