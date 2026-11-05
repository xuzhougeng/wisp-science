using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeFolderActionsTests
{
    public static async Task RunAsync(string fixtures)
    {
        using var fixture = JsonDocument.Parse(File.ReadAllText(Path.Combine(fixtures, "../../native-projects/v1/folder-actions.json")));
        var deleted = fixture.RootElement.GetProperty("delete").GetProperty("result").Deserialize<NativeFolderDeleteResult>()!;
        var order = fixture.RootElement.GetProperty("reorder").GetProperty("result").Deserialize<NativeFolderOrderResult>()!;
        Check(deleted.ProjectId == "research-1" && deleted.FolderId == "methods" && deleted.Deleted
            && order.ProjectId == deleted.ProjectId && order.Ids.SequenceEqual(["results", "methods"]), "shared folder acknowledgement fixtures");
        var fake = new Fake(); using var groups = new WorkspaceSessionGroups(fake, "p"); await groups.LoadAsync();
        var menu = NativeSessionMenuEntries.Folder("a", groups);
        Check(menu.Select(e => e.LabelKey).SequenceEqual(["ctx.rename_folder", "ctx.delete_folder"]) && menu[1].Danger, "folder menu order and danger presentation");
        BrowserSession[] rows = [new("s", "p", "Session", 1, "complete", "a"), new("u", "p", "Ungrouped", 2, "complete")];
        NativeFolderDrop? Plan(string kind, string id, string? target) => NativeFolderDrop.Plan(new("p", kind, id), "p", groups, rows, target);
        Check(Plan("folder", "a", "c") is { After: true, Order: ["b", "c", "a"] }
            && Plan("folder", "c", "a") is { After: false, Order: ["c", "a", "b"] }, "reorder inserts at target old index in either direction");
        Check(Plan("folder", "a", null) == null && Plan("folder", "a", "a") == null && Plan("folder", "missing", "a") == null, "groups cannot be dropped into ungrouped, themselves or an unknown folder");
        Check(Plan("session", "s", "b") is { SessionId: "s", FolderId: "b", Order: null }
            && Plan("session", "s", null) is { FolderId: null } && Plan("session", "u", null) == null, "moves and ungrouping reject no-ops");
        Check(Plan("session", "missing", "b") == null && Plan("session", "s", "missing") == null
            && NativeFolderDrop.Plan(new("other", "session", "s"), "p", groups, rows, "b") == null, "drag identity is scoped to loaded project membership");
        groups.Group = "date"; Check(Plan("session", "s", "b") == null, "date mode cannot reassign folders"); groups.Group = "folder";
        groups.Selecting = true; Check(Plan("folder", "a", "b") == null, "selection mode does not start moves"); groups.Selecting = false;
        Check(await groups.ReorderFoldersAsync(["c", "a", "b"]) && groups.Folders.Select(f => f.Id).SequenceEqual(["c", "a", "b"]), "matching order acknowledgement updates the displayed order");
        var before = fake.Writes; await groups.ReorderFoldersAsync(["c", "a", "b"]); await groups.ReorderFoldersAsync(["a", "a", "b"]);
        Check(fake.Writes == before, "same and duplicate orders never write");
        Check(await groups.DeleteFolderAsync("b") && groups.Folders.All(f => f.Id != "b") && fake.LastProject == "p", "delete only removes the acknowledged owned folder");
        groups.BeginFolderEdit("a"); groups.Draft = "  Renamed  "; Check(await groups.SaveAsync() && groups.Folders.Single(f => f.Id == "a").Name == "Renamed", "rename trims and reads back saved name");
        groups.BeginFolderEdit(null); groups.Draft = "  New  "; Check(await groups.SaveAsync() && groups.Folders.Any(f => f.Name == "New"), "create validates acknowledgement and refreshes groups");

        var lost = new Fake { FailWrites = true }; using var uncertain = new WorkspaceSessionGroups(lost, "p"); await uncertain.LoadAsync();
        uncertain.BeginFolderEdit(null); uncertain.Draft = "Keep this draft"; await uncertain.SaveAsync(); await uncertain.SaveAsync();
        Check(uncertain.FolderSaveUnconfirmed && !uncertain.CanSave && uncertain.Draft == "Keep this draft" && lost.Writes == 1, "uncertain create keeps draft and cannot be replayed by repeated Enter");
        Check(uncertain.FolderFailureDetail == "Response lost", "folder dialogs receive detail without a hard-coded language prefix");
        await uncertain.DeleteFolderAsync("a"); await uncertain.DeleteFolderAsync("a");
        Check(uncertain.FolderDeleteUnconfirmed("a") && uncertain.Folders.Length == 3 && lost.Writes == 2, "uncertain delete leaves rows and disables another request");
        await uncertain.ReorderFoldersAsync(["c", "b", "a"]); await uncertain.ReorderFoldersAsync(["c", "b", "a"]);
        Check(uncertain.FolderOrderUnconfirmed && lost.Writes == 3, "uncertain reorder is not replayed");

        var wrong = new Fake { WrongAck = true }; using var mismatched = new WorkspaceSessionGroups(wrong, "p"); await mismatched.LoadAsync();
        await mismatched.DeleteFolderAsync("a"); await mismatched.ReorderFoldersAsync(["c", "b", "a"]);
        Check(mismatched.FolderDeleteUnconfirmed("a") && mismatched.FolderOrderUnconfirmed && mismatched.Folders[0].Id == "a", "mismatched project acknowledgements cannot change sidebar state");
        var slow = new Fake { Pending = new() }; using var closed = new WorkspaceSessionGroups(slow, "p"); await closed.LoadAsync();
        var pending = closed.DeleteFolderAsync("a"); closed.Dispose(); slow.Pending.SetResult(JsonSerializer.SerializeToNode(new NativeFolderDeleteResult("p", "a", true)));
        Check(!await pending && closed.Folders.Length == 3, "late delete reply cannot mutate a closed project model");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public ProjectFolder[] Folders = [new("a", "Alpha"), new("b", "Beta"), new("c", "Gamma")];
        public int Writes; public string? LastProject; public bool FailWrites, WrongAck; public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            LastProject = projectId;
            if (command == "native_project_folders") return Task.FromResult(JsonSerializer.SerializeToNode(Folders, ConversationSnapshot.JsonOptions));
            Writes++; if (FailWrites) throw new IOException("Response lost"); if (Pending != null) return Pending.Task;
            if (command == "native_project_folder_delete")
            {
                var id = args["folder_id"]!.GetValue<string>(); if (!WrongAck) Folders = Folders.Where(f => f.Id != id).ToArray();
                return Task.FromResult(JsonSerializer.SerializeToNode(new NativeFolderDeleteResult(WrongAck ? "other" : projectId!, id, true)));
            }
            if (command == "native_project_folders_reorder")
            {
                var ids = args["ids"]!.Deserialize<string[]>()!;
                if (!WrongAck) { var map = Folders.ToDictionary(f => f.Id); Folders = ids.Select(id => map[id]).ToArray(); }
                return Task.FromResult(JsonSerializer.SerializeToNode(new NativeFolderOrderResult(WrongAck ? "other" : projectId!, ids)));
            }
            var name = args["name"]!.GetValue<string>();
            if (command == "native_project_folder_create")
            {
                var created = new ProjectFolder("new", name); Folders = Folders.Append(created).ToArray();
                return Task.FromResult(JsonSerializer.SerializeToNode(created, ConversationSnapshot.JsonOptions));
            }
            var renamed = args["folder_id"]!.GetValue<string>(); Folders = Folders.Select(f => f.Id == renamed ? f with { Name = name } : f).ToArray();
            return Task.FromResult<JsonNode?>(JsonValue.Create(true));
        }
    }
}
