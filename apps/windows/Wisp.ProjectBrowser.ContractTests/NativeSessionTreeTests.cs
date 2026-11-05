using System.Text.Json;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSessionTreeTests
{
    public static void Run(string fixtureDirectory)
    {
        using var fixture = JsonDocument.Parse(File.ReadAllText(Path.Combine(fixtureDirectory, "relationships.json")));
        var sessions = fixture.RootElement.GetProperty("sessions").Deserialize<BrowserSession[]>()!;
        var tree = NativeSessionTree.Build(sessions);
        Check(tree.Roots.Select(s => s.Id).SequenceEqual(["main", "orphaned"]), "only visible families nest; detached branch stays visible");
        Check(tree.ChildrenOf("main").Select(s => s.Id).SequenceEqual(["active", "merged", "nested", "child"]), "branches flatten and subagents follow them");
        Check(sessions[1].BranchedFrom == "main" && sessions[2].BranchState == "merged" && sessions[4].DispatchedFrom == "main", "shared DTO retains relationship metadata");
        var baseRow = sessions[0];
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>());
        foreach (var (title, expected) in new[] { ("Branch: analysis", "A"), ("实验记录", "实"), ("🧪 experiment", "🧪") })
            Check(NativeSessionTree.RailInitial(baseRow with { Title = title, BranchState = "active" }, locale) == expected,
                "the rail uses the displayed conversation title and never splits its first character");
        BrowserSession Row(string id, string? parent = null, string? folder = null, string? agent = null) => baseRow with { Id = id, BranchedFrom = parent, FolderId = folder, DispatchedFrom = agent };
        var edge = NativeSessionTree.Build([Row("main"), Row("missing", "absent"), Row("moved", "main", "folder"), Row("cycle1", "cycle2"), Row("cycle2", "cycle1"), Row("sub", folder: "legacy", agent: "main"), Row("nested", "moved", "folder")]);
        Check(edge.Roots.Select(s => s.Id).SequenceEqual(["main", "missing", "moved", "cycle1", "cycle2"]), "missing source, moved folder and cycles never hide conversations");
        Check(edge.ChildrenOf("main").Single().Id == "sub" && edge.ChildrenOf("moved").Single().Id == "nested", "subagents ignore legacy folder, branch ancestry stops at folder boundary");
        var pinned = NativeSessionTree.Build([Row("main") with { Pinned = true }, Row("branch", "main"), Row("pin-child", "branch") with { Pinned = true }]);
        Check(pinned.Roots.Length == 3 && pinned.Children.Count == 0, "pins lift out before nesting");
        var cross = NativeSessionTree.Build([Row("main"), Row("foreign", "main") with { ProjectId = "other" }]);
        Check(cross.Roots.Length == 2, "never join a different project's family");
        var nesting = new NativeSessionNesting(); var children = tree.ChildrenOf("main");
        Check(!nesting.Expanded("main", children, "unrelated") && nesting.Expanded("main", children, "main")
            && nesting.Expanded("main", children, "nested"), "family auto-opens only when in use");
        Check(nesting.Expanded("main", [children[0] with { Status = "running" }], null)
            && nesting.Expanded("main", [children[0] with { Status = "needs_you" }], null), "busy children reveal their family");
        nesting.SetExpanded("main", false);
        Check(!nesting.Expanded("main", [children[0] with { Status = "running" }], "nested"), "explicit collapse survives activity and navigation");
        nesting.SetExpanded("main", true); Check(nesting.Expanded("main", children, null), "explicit expansion survives rerender");
        Check(NativeSessionTree.Title(sessions[1] with { Title = "Branch: Normalize" }, locale) == "Normalize"
            && NativeSessionTree.Title(baseRow with { Title = "Branch: Authored title" }, locale) == "Branch: Authored title", "only actual branches strip generated prefix");
        Check(NativeSessionTree.Kind(sessions[1]) == "branch" && NativeSessionTree.Kind(sessions[4]) == "bot" && NativeSessionTree.Kind(baseRow) == null, "semantic row icons");
        foreach (var kind in sessions.Select(NativeSessionTree.Kind).OfType<string>().Distinct())
            Check(File.Exists(Path.Combine(fixtureDirectory, "../../../apps/macos/Sources/WispProjectBrowserUI/Resources", $"icon-{kind}.svg")), "every relationship icon is packaged from the shared source");
        var old = JsonSerializer.Deserialize<BrowserSession>("{\"id\":\"old\",\"project_id\":\"p\",\"title\":\"Old\",\"ts\":1,\"status\":\"complete\"}")!;
        Check(old.BranchedFrom == null && old.BranchState == null && old.DispatchedFrom == null, "older response remains compatible");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
