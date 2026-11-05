using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>The WebView's one-level conversation families. Relationships never
/// hide a row whose source is absent, pinned separately, in another folder or cyclic.</summary>
public sealed record NativeSessionTree(BrowserSession[] Roots, IReadOnlyDictionary<string, BrowserSession[]> Children)
{
    public BrowserSession[] ChildrenOf(string id) => Children.TryGetValue(id, out var children) ? children : [];
    public static NativeSessionTree Build(IEnumerable<BrowserSession> sessions)
    {
        var list = sessions.ToArray();
        var visible = list.Where(s => s.Pinned != true).ToDictionary(s => s.Id);
        var roots = new List<BrowserSession>(); var children = new Dictionary<string, List<BrowserSession>>();
        foreach (var session in list)
        {
            var current = session; var hops = 0;
            if (session.Pinned != true)
                while ((current.BranchedFrom ?? current.DispatchedFrom) is { } parent && visible.TryGetValue(parent, out var source))
                {
                    if (source.ProjectId != current.ProjectId || current.DispatchedFrom == null && source.FolderId != current.FolderId) break;
                    current = source;
                    if (++hops > 16) { current = session; break; }
                }
            if (current.Id == session.Id) roots.Add(session);
            else
            {
                if (!children.TryGetValue(current.Id, out var family)) children[current.Id] = family = [];
                family.Add(session);
            }
        }
        return new(roots.ToArray(), children.ToDictionary(pair => pair.Key,
            pair => pair.Value.Where(s => s.DispatchedFrom == null).Concat(pair.Value.Where(s => s.DispatchedFrom != null)).ToArray()));
    }
    public static string Title(BrowserSession session, NativeLocalization locale)
    {
        if (string.IsNullOrWhiteSpace(session.Title)) return locale.Text("sidebar.untitled");
        return session.BranchState != null && session.Title.StartsWith("Branch: ", StringComparison.Ordinal)
            ? session.Title[8..] : session.Title;
    }
    public static string RailInitial(BrowserSession session, NativeLocalization locale)
        => System.Globalization.StringInfo.GetNextTextElement(Title(session, locale)).ToUpperInvariant();
    public static string? Kind(BrowserSession session) => session.DispatchedFrom != null ? "bot" : session.BranchState != null ? "branch" : null;
}

/// <summary>Explicit disclosure choices outlive sidebar redraws, but not the project.</summary>
public sealed class NativeSessionNesting
{
    private readonly Dictionary<string, bool> choices = [];
    public bool Expanded(string parent, IEnumerable<BrowserSession> children, string? active) =>
        choices.TryGetValue(parent, out var choice) ? choice : active == parent
            || children.Any(s => s.Id == active || s.Status is "running" or "needs_you");
    public void SetExpanded(string parent, bool expanded) => choices[parent] = expanded;
}
