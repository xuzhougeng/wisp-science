using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeWorkspaceActionsTests
{
    public static async Task RunAsync()
    {
        ArtifactGroups();
        PinnedSections();
        await SessionMutationsAsync();
        Console.WriteLine("Native workspace actions (W2/W3) contract tests passed.");
    }

    private static void ArtifactGroups()
    {
        var artifacts = new[]
        {
            new NativePanelArtifact("a1", "fig.png", "image/png", "p1", null, 1, "fig.png"),
            new NativePanelArtifact("a2", "fit.R", "text/x-r", "p2", null, 2, "fit.R"),
            new NativePanelArtifact("a3", "table.csv", "text/csv", "p3", null, 3, "table.csv"),
            new NativePanelArtifact("a4", "paper.pdf", "application/pdf", "p4", null, 4, "paper.pdf"),
            new NativePanelArtifact("a5", "raw.dat", "", "p5", null, 5, "raw.dat"),
            new NativePanelArtifact("a6", "fig2.png", "image/png", "p6", null, 6, "fig2.png"),
        };
        var groups = NativeArtifactGroups.Collect(artifacts);
        Require(groups.Count == 5, "five distinct type groups");
        Require(groups[0].Label.StartsWith("图片", StringComparison.Ordinal) && groups[0].Items.Count == 2, "images group first with count");
        Require(groups[1].Label == "PDF 文档", "pdf rank before text");
        Require(groups.Single(g => g.Label == "其他").Items.Count == 1, "unknown kind grouped as other");
        Require(NativeArtifactGroups.Collect(artifacts, "fig").Sum(g => g.Items.Count) == 2, "name filter reaches groups");
        Require(NativeArtifactGroups.Collect(artifacts, "没有").Count == 0, "no match yields no groups");
        Require(NativeArtifactGroups.Label("IMAGE/PNG").Label.StartsWith("图片", StringComparison.Ordinal), "kind label is case-insensitive");
    }

    private static void PinnedSections()
    {
        var pinned = new BrowserSession("p1", "proj", "置顶会话", 100, "done", null, Pinned: true);
        var normal = new BrowserSession("n1", "proj", "普通会话", 200, "done", null, Pinned: false);
        var unknown = new BrowserSession("u1", "proj", "旧数据会话", 300, "done", null, Pinned: null);

        var groups = new WorkspaceSessionGroups(new RecordingSettings(), "proj") { Group = "none", Sort = "newest" };
        var sections = groups.Sections([normal, pinned, unknown]);
        Require(sections[0].Title == "已置顶" && sections[0].Sessions.Single().Id == "p1", "pinned section leads");
        Require(sections[1].Sessions.Select(s => s.Id).SequenceEqual(["u1", "n1"]), "unknown pin keeps its newest order");

        var dated = new WorkspaceSessionGroups(new RecordingSettings(), "proj") { Group = "date" };
        sections = dated.Sections([pinned, normal]);
        Require(sections[0].Title == "已置顶" && sections.Skip(1).All(s => !s.Sessions.Any(x => x.Id == "p1")),
            "pinned section leads in date grouping too");

        var foldered = new WorkspaceSessionGroups(new RecordingSettings(), "proj") { Group = "folder" };
        sections = foldered.Sections([pinned, normal]);
        Require(sections[0].Title == "已置顶", "pinned section leads in folder grouping");

        var unpinned = new WorkspaceSessionGroups(new RecordingSettings(), "proj");
        sections = unpinned.Sections([normal, unknown]);
        Require(sections.Length == 1 && sections[0].Title == "会话", "no pinned rows means no pinned section");
    }

    private static async Task SessionMutationsAsync()
    {
        var client = new RecordingSettings();
        var groups = new WorkspaceSessionGroups(client, "proj");
        Require(await groups.RenameAsync("s1", "  新名字  "), "rename succeeds");
        Require(client.Commands.Single(c => c.Command == "native_conversation_rename").Arguments["session_id"]!.GetValue<string>() == "s1"
            && client.Commands.Single(c => c.Command == "native_conversation_rename").Arguments["title"]!.GetValue<string>() == "新名字",
            "rename trims and sends the session id");

        client.Fail = true;
        Require(!await groups.RenameAsync("s1", "再改"), "failed rename reports failure");
        Require(groups.Error!.Contains("不会自动重试"), "rename failure surfaces no-retry wording");
        Require(!await groups.RenameAsync("s1", "再改一次"), "a second explicit save still reports failure");
        Require(client.Commands.Count(c => c.Command == "native_conversation_rename") == 3,
            "explicit user actions invoke once each; only automatic retries are forbidden");
        client.Fail = false;

        Require(!await groups.RenameAsync("s1", "   "), "empty rename is rejected locally");
        Require(client.Commands.Count(c => c.Command == "native_conversation_rename") == 3, "empty rename never reaches the host");

        Require(await groups.PinAsync("s1", true), "pin succeeds");
        var pin = client.Commands.Last(c => c.Command == "native_conversation_pin");
        Require(pin.Arguments["pinned"]!.GetValue<bool>(), "pin sends true");
        Require(await groups.PinAsync("s1", false), "unpin succeeds");
        Require(!client.Commands.Last(c => c.Command == "native_conversation_pin").Arguments["pinned"]!.GetValue<bool>(), "unpin sends false");

        Require(await groups.DeleteAsync("s1"), "delete succeeds");
        Require(client.Commands.Last(c => c.Command == "native_conversation_delete").Arguments["session_id"]!.GetValue<string>() == "s1",
            "delete sends the session id");

        client.Fail = true;
        Require(!await groups.DeleteAsync("s2"), "failed delete reports failure");
        Require(!await groups.DeleteAsync("s2"), "failed delete is not retried automatically");
    }

    private sealed record RecordedCommand(string Command, JsonObject Arguments);

    private sealed class RecordingSettings : INativeSettingsClient
    {
        public List<RecordedCommand> Commands = [];
        public bool Fail;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null,
            CancellationToken cancellationToken = default)
        {
            Commands.Add(new RecordedCommand(command, arguments));
            if (Fail) throw new IOException("response lost");
            if (command == "native_project_folders") return Task.FromResult<JsonNode?>(new JsonArray());
            return Task.FromResult<JsonNode?>(null);
        }
    }

    private static void Require(bool condition, string label)
    {
        if (!condition) throw new InvalidOperationException("Native workspace action drift: " + label);
    }
}
