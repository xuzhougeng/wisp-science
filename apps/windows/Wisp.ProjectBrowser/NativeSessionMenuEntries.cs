using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeSessionMenuEntry(string Id, string LabelKey, string? Payload = null,
    bool Enabled = true, bool Danger = false, NativeSessionMenuEntry[]? Children = null, string? LiteralLabel = null, bool Visible = true);

/// <summary>Shared ordering for the row's context and overflow entry points.</summary>
public static class NativeSessionMenuEntries
{
    public static NativeSessionMenuEntry[] Exploration(NativeExplorationSummary summary) =>
    [
        new("open", "exploration.open"),
        .. (summary.Exploration.Status == "active" ? new[] { new NativeSessionMenuEntry("promote", "exploration.select_as_mainline") } : []),
        new("preview", "exploration.view_diff"),
        .. (summary.Exploration.Status is "active" or "failed" ? new[] { new NativeSessionMenuEntry("discard", "exploration.discard", Danger: true) } : []),
    ];
    public static NativeSessionMenuEntry[] Folder(string id, WorkspaceSessionGroups groups) =>
        [new("rename", "ctx.rename_folder", id, Enabled: !groups.Closed && !groups.Busy),
         new("delete", "ctx.delete_folder", id, Enabled: groups.CanDeleteFolder(id), Danger: true)];
    public static NativeSessionMenuEntry[] Conversation(BrowserSession session, WorkspaceSessionGroups groups, bool? stalePrompt = false) =>
    [
        new("copy", "ctx.copy_title"), new("open", "ctx.open_session"),
        .. (stalePrompt != false ? new[] { new NativeSessionMenuEntry("reload_rules", "ctx.reload_rules", Enabled: !groups.Busy, Visible: stalePrompt == true) } : []),
        .. (session.BranchState != null && session.BranchState != "merged"
            ? new[] { new NativeSessionMenuEntry("merge_branch", "branch.merge", Enabled: !groups.Busy) } : []),
        new("pin", session.Pinned == true ? "ctx.unpin_session" : "ctx.pin_session", Enabled: session.Pinned != null && !groups.Busy),
        new("shelve", "session.shelve", Enabled: groups.CanShelve(session.Id)),
        new("rename", "ctx.rename_session", Enabled: !groups.Busy),
        new("move", "ctx.move_to_prefix", Enabled: !groups.Busy && !groups.MoveUnconfirmed, Children:
            new[] { new NativeSessionMenuEntry("move", "ctx.move_to_ungrouped") }.Concat(groups.Folders.Select(folder =>
                new NativeSessionMenuEntry("move", "folder.untitled", folder.Id, LiteralLabel: string.IsNullOrWhiteSpace(folder.Name) ? null : folder.Name))).ToArray()),
        new("copy_project", "ctx.copy_to_project", Enabled: !groups.Busy),
        new("move_project", "ctx.move_to_project", Enabled: !groups.Busy),
        new("export", "ctx.export_session", Enabled: !groups.Busy),
        new("export_debug", "ctx.export_debug_request", Enabled: !groups.Busy),
        new("delete", session.BranchState != null ? "branch.delete" : "ctx.delete_session", Enabled: !groups.Busy, Danger: true)
    ];
    public static NativeSessionMenuEntry[] Shelved(bool canRestore) =>
        [new("open", "ctx.open_session"), new("restore", "session.restore", Enabled: canRestore)];
}
