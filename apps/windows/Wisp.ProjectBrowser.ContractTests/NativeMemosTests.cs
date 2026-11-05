using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeMemosTests
{
    public static async Task RunAsync()
    {
        var host = new Host(); var client = new NativeMemosClient(host); using var model = new WorkspaceMemosModel(client, "p");
        await model.LoadAsync(); Check(model.Memos.Select(m => m.Id).SequenceEqual(["pinned", "new", "old"]), "pinned first, then updated descending");
        model.Query = "GPU"; Check(model.Filtered.Single().Id == "old", "search body ignores case"); model.Query = "";
        model.New(); var draft = model.Draft!; Check(!draft.Dirty && !draft.CanSave, "empty draft cannot save");
        draft.Title = "Title only"; draft.Body = " \n"; await model.SaveAsync(); Check(host.Writes == 0, "blank body never writes");
        draft.Body = "private body"; Check(model.Escape() && model.ConfirmDiscard, "dirty Escape asks first");
        Check(model.Escape() && !model.ConfirmDiscard && ReferenceEquals(draft, model.Draft), "second Escape retains editor and draft");
        model.Cancel(); model.KeepEditing(); Check(ReferenceEquals(model.Draft, draft), "keep editing preserves draft");
        await model.SaveAsync(); Check(model.Draft == null && model.Memos.Any(m => m.Body == "private body") && host.Writes == 1, "explicit Save persists once");
        var saved = model.Memos.Single(m => m.Body == "private body"); model.Edit(saved); model.Draft!.Body = "local only"; model.Discard();
        Check(model.Memos.Single(m => m.Id == saved.Id).Body == "private body" && host.Writes == 1, "cancel never edits stored note");
        const string multiline = "First line\n第二行\n\nFinal line";
        model.Edit(saved); model.Draft!.Body = multiline; await model.SaveAsync();
        saved = model.Memos.Single(m => m.Id == saved.Id); model.Edit(saved);
        Check(model.Draft!.Body == multiline && !model.Draft.Dirty, "saved multi-line body reopens unchanged, including blank lines");
        var cleanWrites = host.Writes; Check(model.Escape() && model.Draft == null && host.Writes == cleanWrites, "immediate Escape closes an unchanged editor without saving");
        await model.PinAsync(saved); Check(model.Memos[0].Id == saved.Id && model.Memos[0].Pinned, "pin reorders confirmed snapshot");
        Check(WorkspaceMemosModel.Heading(saved with { Title = " ", Body = "\n first line \nsecond" }) == "first line", "heading fallback");
        var windowsDraft = new NativeMemoDraft(body: "first\rsecond\r\rlast");
        Check(windowsDraft.Body == "first\nsecond\n\nlast" && !windowsDraft.Dirty, "WinUI CR-only body normalizes without becoming dirty");
        Check(WorkspaceMemosModel.Heading(saved with { Title = "", Body = "\r first\rsecond" }) == "first", "Windows line endings cannot turn the heading into the whole body");
        Check(WorkspaceMemosModel.ComposeInsert("  \n", "body") == "body" && WorkspaceMemosModel.ComposeInsert("current\n", "body") == "current\n\nbody", "insert uses body only and preserves composer");
        model.New(); model.Draft!.Body = "uncertain"; host.Fail = true; var before = host.Writes; await model.SaveAsync(); await model.SaveAsync();
        Check(model.Uncertain && model.SaveError != null && model.Draft.Body == "uncertain" && host.Writes == before + 1, "failed write preserves draft and is never replayed");
        host.Fail = false; model.Discard(); host.WrongOwner = true;
        await Throws(() => client.ListAsync("p")); host.WrongOwner = false; host.WrongId = true;
        await Throws(() => client.UpdateAsync("p", new("old", Pinned: true))); host.WrongId = false;
        var commandCount = host.Commands.Count;
        await model.DeleteAsync(saved with { ProjectId = "other" }); Check(host.Commands.Count == commandCount, "foreign memo cannot issue a delete");
        host.WrongId = true; await model.DeleteAsync(model.Memos.Single(m => m.Id == "old"));
        Check(model.Uncertain && model.Memos.Any(m => m.Id == "old"), "unconfirmed delete retains the visible note");
        host.WrongId = false; await model.LoadAsync();
        Check(!model.Uncertain && model.Memos.All(m => m.Id != "old"), "explicit reload resolves an uncertain delete without replay");
        host.Hold = new(); var closing = new WorkspaceMemosModel(client, "p"); var read = closing.LoadAsync(); closing.Dispose(); host.Hold.SetResult(); await read;
        Check(closing.Memos.Length == 0 && closing.Draft == null, "late response cannot restore a departed project");
        Check(host.Commands.All(c => c.StartsWith("native_project_memo")), "memo actions never call a model or create a session");
        var tabs = new NativePanelTabs("[\"files\",\"memos\"]", "memos", ["files", "memos"]);
        Check(tabs.Selected == "memos", "memo tab is available without a session"); tabs.Remove("memos"); Check(tabs.Selected == "files", "closing selected memo tab returns to files");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static async Task Throws(Func<Task> run) { try { await run(); } catch (InvalidDataException) { return; } throw new Exception("Scope mismatch accepted"); }
    private sealed class Host : INativeSettingsClient
    {
        public int Writes; public bool Fail, WrongOwner, WrongId; public TaskCompletionSource? Hold; public List<string> Commands = [];
        private List<NativeMemo> memos = [new("old", "p", "Server", "ssh gpu02", false, 1, 1), new("pinned", "p", "Pinned", "first", true, 2, 2), new("new", "p", "New", "last", false, 3, 3)];
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(projectId == "p" && args["session_id"] == null, "explicit project and no implicit conversation"); Commands.Add(command);
            if (Hold is { } hold) await hold.Task;
            if (command == "native_project_memos") return JsonSerializer.SerializeToNode(new NativeMemoList(NativeMemosClient.Schema, "p", memos.Select(m => WrongOwner ? m with { ProjectId = "other" } : m).ToArray()), ConversationSnapshot.JsonOptions);
            Writes++; if (Fail) throw new IOException("Uncertain response");
            if (command == "native_project_memo_delete")
            {
                var id = args["memo_id"]!.GetValue<string>(); memos.RemoveAll(m => m.Id == id);
                return JsonSerializer.SerializeToNode(new NativeDeletedMemo(NativeMemosClient.Schema, "p", WrongId ? "wrong" : id), ConversationSnapshot.JsonOptions);
            }
            NativeMemo saved;
            if (command == "native_project_memo_add") { var input = args["input"]!.Deserialize<NativeMemoInput>(ConversationSnapshot.JsonOptions)!; saved = new("added", "p", input.Title, input.Body, false, 9, 9); memos.Add(saved); }
            else { var patch = args["patch"]!.Deserialize<NativeMemoPatch>(ConversationSnapshot.JsonOptions)!; var old = memos.Single(m => m.Id == patch.Id); saved = old with { Title = patch.Title ?? old.Title, Body = patch.Body ?? old.Body, Pinned = patch.Pinned ?? old.Pinned, UpdatedAt = 10 }; memos.Remove(old); memos.Add(saved); }
            return JsonSerializer.SerializeToNode(new NativeMemoResult(NativeMemosClient.Schema, "p", WrongId ? saved with { Id = "wrong" } : saved), ConversationSnapshot.JsonOptions);
        }
    }
}
