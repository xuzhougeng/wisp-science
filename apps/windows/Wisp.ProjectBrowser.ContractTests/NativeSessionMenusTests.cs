using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSessionMenusTests
{
    public static async Task RunAsync()
    {
        var client = new Fake(); using var groups = new WorkspaceSessionGroups(client, "p");
        client.Handler = (command, _) => Task.FromResult<JsonNode?>(JsonSerializer.SerializeToNode(new[] { new ProjectFolder("f", "Research"), new ProjectFolder("empty", " ") }, ConversationSnapshot.JsonOptions));
        await groups.LoadAsync();
        var session = new BrowserSession("s", "p", "Study", 1, "complete", null, false);
        var menu = NativeSessionMenuEntries.Conversation(session, groups);
        Check(menu.Select(e => e.Id).SequenceEqual(["copy", "open", "pin", "shelve", "rename", "move", "copy_project", "move_project", "export", "export_debug", "delete"]), "row and overflow share WebView action ordering");
        Check(menu.Last().Danger && menu[2].LabelKey == "ctx.pin_session", "delete danger and pin label");
        Check(NativeSessionMenuEntries.Conversation(session with { BranchState = "merged" }, groups).Last().LabelKey == "branch.delete", "branch row retains the WebView delete label");
        Check(NativeSessionMenuEntries.Conversation(session with { BranchState = "active" }, groups)[2].Id == "merge_branch"
            && NativeSessionMenuEntries.Conversation(session with { BranchState = "merged" }, groups).All(e => e.Id != "merge_branch"), "unmerged branches offer Merge back immediately after Open; merged branches do not");
        Check(NativeSessionMenuEntries.Conversation(session with { Pinned = true }, groups)[2].LabelKey == "ctx.unpin_session"
            && !NativeSessionMenuEntries.Conversation(session with { Pinned = null }, groups)[2].Enabled, "exact known pin state");
        var move = menu.Single(e => e.Id == "move").Children!;
        Check(move.Select(e => e.Payload).SequenceEqual([null, "f", "empty"]) && move[0].LabelKey == "ctx.move_to_ungrouped"
            && move[1].LiteralLabel == "Research" && move[2].LiteralLabel == null, "ungrouped first and empty folder gets shared fallback");
        Check(NativeSessionMenuEntries.Shelved(false) is [{ Id: "open", Enabled: true }, { Id: "restore", Enabled: false }], "shelf context permits reading while restore is unavailable");
        client.Handler = (_, args) => Task.FromResult<JsonNode?>(JsonSerializer.SerializeToNode(new SessionShelveResult("p", args["session_id"]!.ToString(), true)));
        Check(await groups.ShelveAsync("s"), "shelve accepts matching acknowledgement");
        Check(client.Calls.Last().Project == "p" && client.Calls.Last().Args["shelved"]!.GetValue<bool>(), "explicit shelf scope and requested state");
        client.Handler = (_, _) => Task.FromResult<JsonNode?>(JsonSerializer.SerializeToNode(new SessionShelveResult("other", "x", true)));
        Check(!await groups.ShelveAsync("x") && !groups.CanShelve("x") && groups.CanShelve("s"), "mismatched acknowledgement blocks only unresolved shelf target");
        var calls = client.Calls.Count; Check(!await groups.ShelveAsync("x") && calls == client.Calls.Count, "unconfirmed shelf cannot be replayed");
        groups.Selected.Add("unrelated"); client.Handler = (_, _) => Task.FromResult<JsonNode?>(JsonValue.Create(true));
        Check(await groups.MoveSessionAsync("s", null) && groups.Selected.SetEquals(["unrelated"]), "row move does not borrow or clear bulk selection");
        client.Handler = (_, _) => Task.FromResult<JsonNode?>(null);
        Check(!await groups.MoveSessionAsync("s", "f") && groups.MoveUnconfirmed, "row move requires positive acknowledgement");

        var renameClient = new Fake(); var pending = new TaskCompletionSource<JsonNode?>(); renameClient.Handler = (_, _) => pending.Task;
        using var rename = new WorkspaceSessionRename(renameClient, "p", "s", "Old title");
        rename.Title = "   "; Check(!rename.CanSave && !await rename.SaveAsync() && renameClient.Calls.Count == 0, "blank title is local only");
        rename.Title = "  新标题  "; var save = rename.SaveAsync();
        Check(!await rename.SaveAsync() && renameClient.Calls.Single().Args["title"]!.ToString() == "新标题", "trimmed rename and duplicate submission guard");
        pending.SetResult(null); Check(await save && rename.Saved && !rename.CanSave && rename.Title == "新标题", "void backend acknowledgement completes rename");
        using var failed = new WorkspaceSessionRename(renameClient, "p", "s", "Keep this draft");
        renameClient.Handler = (_, _) => throw new IOException("Lost acknowledgement");
        Check(!await failed.SaveAsync() && failed.Title == "Keep this draft" && failed.MutationUnconfirmed && !failed.CanSave, "unknown rename retains draft without replay");
        using var closed = new WorkspaceSessionRename(renameClient, "p", "s", "Pending");
        pending = new(); renameClient.Handler = (_, _) => pending.Task; var late = closed.SaveAsync(); closed.Dispose(); pending.SetResult(null);
        Check(!await late && !closed.Saved, "closing invalidates late rename reply");
        Check(renameClient.Calls.All(c => c.Project == "p" && c.Args["session_id"]!.ToString() == "s"), "rename never loses project/session scope");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<string, JsonObject, Task<JsonNode?>> Handler = (_, _) => Task.FromResult<JsonNode?>(null);
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls.Add((command, args, projectId)); return Handler(command, args); }
    }
}
