using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeFolderDrop(string? SessionId, string? FolderId, string[]? Order, bool After)
{
    public static NativeFolderDrop? Plan(NativeSessionDrag? drag, string project,
        WorkspaceSessionGroups groups, IReadOnlyList<BrowserSession> sessions, string? target)
    {
        if (drag is not { Valid: true } || drag.ProjectId != project || groups.Closed || groups.Busy || groups.Selecting || groups.Group != "folder") return null;
        if (target != null && !groups.Folders.Any(f => f.Id == target)) return null;
        if (drag.Kind == "session")
        {
            var session = sessions.FirstOrDefault(s => s.Id == drag.Id && s.ProjectId == project);
            return session == null || session.FolderId == target || groups.MoveUnconfirmed ? null : new(session.Id, target, null, false);
        }
        if (target == null || !groups.CanReorderFolders) return null;
        var ids = groups.Folders.Select(f => f.Id).ToList(); var from = ids.IndexOf(drag.Id); var to = ids.IndexOf(target);
        if (from < 0 || to < 0 || from == to) return null;
        ids.RemoveAt(from); ids.Insert(to, drag.Id);
        return new(null, target, ids.ToArray(), from < to);
    }
}
