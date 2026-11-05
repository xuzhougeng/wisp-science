using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSessionGroupingTests
{
    public static async Task RunAsync()
    {
        var now = DateTimeOffset.Parse("2026-11-01T01:00:00Z");
        var boundary = now.AddDays(-1).ToUnixTimeSeconds();
        BrowserSession Row(string id, long ts, string? folder = null, string? parent = null) =>
            new(id, "p", id, ts, "complete", FolderId: folder, BranchedFrom: parent, BranchState: parent == null ? null : "active");
        using var groups = new WorkspaceSessionGroups(new Folders(), "p");
        Check(groups.Group == "folder", "new windows default to folder grouping");
        await groups.LoadAsync();
        BrowserSession[] sessions = [Row("recent", boundary + 1), Row("old", boundary - 1), Row("exact", boundary),
            Row("ms", boundary * 1000), Row("zero", 0), Row("future", now.AddDays(1).ToUnixTimeMilliseconds()),
            Row("folder", 1, "f"), Row("child", 2, "f", "folder"), Row("missing-folder", 0, "deleted"),
            Row("pin", 0, "f") with { Pinned = true }];
        var tree = groups.Tree(sessions);
        var sections = groups.Sections(tree.Roots, now);
        Check(sections.Select(s => s.LabelKey ?? s.FolderId).SequenceEqual(["sidebar.pinned", "f", "empty", "sidebar.today", "sidebar.earlier"]),
            "pins precede all folders, including empty ones, then rolling date buckets");
        var folder = sections.Single(s => s.FolderId == "f");
        Check(folder.Sessions.Select(s => s.Id).SequenceEqual(["folder"]) && tree.ChildrenOf("folder").Single().Id == "child",
            "folder count and selection use roots, nested rows are independent");
        groups.ToggleSelection(folder.Sessions.Select(s => s.Id));
        Check(groups.Selected.SetEquals(["folder"]), "folder selection excludes child and detached pin");
        Check(sections.Single(s => s.LabelKey == "sidebar.today").Sessions.Select(s => s.Id).SequenceEqual(["recent", "exact", "ms", "future"]),
            "24-hour boundary is inclusive, both timestamp units work, future activity stays today");
        Check(sections.Single(s => s.LabelKey == "sidebar.earlier").Sessions.Select(s => s.Id).SequenceEqual(["old", "zero", "missing-folder"]),
            "older, unknown-time and stale folder references stay readable");
        groups.Group = "date";
        sections = groups.Sections(tree.Roots, now);
        Check(sections.Length == 3 && sections.All(s => s.FolderId == null), "date mode hides folder headers without dropping their sessions");
        groups.Group = "none";
        sections = groups.Sections(tree.Roots, now);
        Check(sections.Length == 2 && sections[0].LabelKey == "sidebar.pinned" && sections[1].LabelKey == null, "ungrouped mode still labels pins");
        Check(sections[1].Sessions.Select(s => s.Id).SequenceEqual(tree.Roots.Where(s => s.Pinned != true).Select(s => s.Id)), "host activity ordering remains intact");
        groups.Sort = "name";
        var tied = groups.Sections([Row("z", 1) with { Title = " alpha " }, Row("a", 2) with { Title = "Alpha" }], now).Single();
        Check(tied.Sessions.Select(s => s.Id).SequenceEqual(["z", "a"]), "name sorting trims, ignores case and preserves backend tie order");
        groups.ToggleFolder("f"); groups.Group = "date"; groups.Group = "folder"; await groups.LoadAsync();
        Check(!groups.FolderExpanded("f") && groups.FolderExpanded("empty"), "collapse survives arrangements and refresh, independently by folder");
        groups.ToggleFolder("f"); Check(groups.FolderExpanded("f"), "second toggle expands");
        using var other = new WorkspaceSessionGroups(new Folders(), "other");
        Check(other.FolderExpanded("f"), "folder disclosure is project-local");
        groups.ToggleSelection(sessions.Select(s => s.Id));
        Check(groups.Selected.Count == sessions.Length, "global select-all includes every loaded nested conversation");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Folders : INativeSettingsClient
    {
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default) =>
            Task.FromResult(JsonSerializer.SerializeToNode(new[] { new ProjectFolder("f", "Methods"), new ProjectFolder("empty", "  ") }, ConversationSnapshot.JsonOptions));
    }
}
