using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeComposerReferencesTests
{
    public static async Task RunAsync(string projectFixture)
    {
        NativeComposerReadingTests.Run();
        await ReadingStateAsync();
        await DocumentReadingStateAsync();
        var fixture = JsonNode.Parse(File.ReadAllText(Path.GetFullPath(Path.Combine(Path.GetDirectoryName(projectFixture)!,
            "../../native-conversations/v1/composer-references.json"))))!;
        var catalog = fixture["response"]!.Deserialize<NativeReferenceCatalog>(ConversationSnapshot.JsonOptions)!;
        Check(catalog.SessionId == "session-a" && catalog.Options.Length == 7 && catalog.Options.All(option => option.Reference.Valid),
            "shared Rust/C# fixture decodes every reference kind");
        Check(catalog.Options.Select(option => option.Reference.Key).Distinct().Count() == 7
            && catalog.Options.Last().Reference.ToJson()["context_id"]!.GetValue<string>() == "context-a", "wire IDs are independent of display labels");
        foreach (var draft in new[] { "", "mail@host", "https://host", "C:/data", "a/b", "@has space" })
            Check(NativeComposerToken.Find(draft, draft.Length) == null, "paths, mail and prose do not open the picker: " + draft);
        var token = NativeComposerToken.Find("看看😀@基因 tail", 7)!;
        Check(token is { Start: 4, End: 7, Kind: "artifact", Query: "基因" }, "UTF-16 caret and CJK boundaries preserve the token");
        Check(token.RemoveFrom("看看😀@基因 tail") == "看看😀 tail", "choosing a reference preserves text after the caret");
        Check(NativeComposerToken.Find("😀@gene", 1) == null, "a split surrogate is never a token boundary");
        Check(NativeComposerToken.Find("#session", 8)?.Kind == "session" && NativeComposerToken.Find("/RNA", 4)?.Kind == "skill", "each trigger uses its catalog");

        var client = new Fake();
        var picker = new NativeComposerReferencesModel(client);
        picker.Bind("project-a", "session-a");
        var oldReply = client.Pending = new();
        var oldSearch = picker.SearchAsync(new(0, 4, "artifact", "old"));
        await client.Entered.Task.WaitAsync(TimeSpan.FromSeconds(3));
        client.Pending = null;
        await picker.SearchAsync(new(0, 4, "artifact", "new"));
        oldReply.SetResult(Catalog("session-a", "old"));
        await oldSearch;
        Check(picker.Options.Single().Reference.Id == "new", "late candidate read cannot replace a newer query");
        Check(client.LastProject == "project-a" && client.LastSession == "session-a", "candidate reads carry project and session identity");

        client.Entered = new(); client.Pending = new();
        var dismissed = picker.SearchAsync(new(0, 4, "artifact", "old"));
        await client.Entered.Task.WaitAsync(TimeSpan.FromSeconds(3));
        picker.Dismiss(); client.Pending.SetResult(Catalog("session-a", "old")); await dismissed;
        Check(picker.Token == null && picker.Options.Length == 0 && !picker.Loading, "Escape prevents a late response from reopening the picker");

        client.Entered = new(); client.Pending = new();
        var switched = picker.SearchAsync(new(0, 4, "artifact", "old"));
        await client.Entered.Task.WaitAsync(TimeSpan.FromSeconds(3));
        picker.Bind("project-b", "session-b"); client.Pending.SetException(new IOException("old failure")); await switched;
        Check(picker.Error == null && picker.Options.Length == 0, "old errors are isolated after navigation");
        client.Pending = null; client.WrongIdentity = true;
        await picker.SearchAsync(new(0, 4, "artifact", "new"));
        Check(picker.Options.Length == 0 && picker.Error != null, "mismatched catalog identity is rejected");

        client = new Fake();
        var model = new WorkspaceConversationModel(new NativeConversationClient(client), client);
        await model.OpenAsync("project-a", "session-a");
        var reference = new NativeReferenceOption(new("artifact", Id: "artifact-exact-id"), "Counts", "Research A");
        Check(model.AddReference(reference) && model.CanSend, "reference-only messages can be sent");
        model.AddReference(reference with { Label = "same ID, new label" });
        Check(model.References.Length == 1, "stable IDs prevent duplicate chips");
        model.Draft = "check these";
        await model.OpenAsync("project-b", "session-b");
        Check(model.References.Length == 0 && model.Draft.Length == 0, "another session starts without the previous references");
        await model.OpenAsync("project-a", "session-a");
        Check(model.References.Single().Reference.Id == "artifact-exact-id" && model.Draft == "check these", "draft and references survive navigation together");
        client.FailSend = true;
        await model.SendAsync();
        Check(model.UncertainSend && model.References.Length == 1 && model.Draft == "check these", "lost send acknowledgement retains references and text");
        Check(client.Sent?["references"]?[0]?["id"]?.GetValue<string>() == "artifact-exact-id"
            && client.Sent?["message"]?.GetValue<string>() == "check these\n\nAttached artifacts: Counts", "send carries both stable ID and persisted display label");
        await model.SendAsync();
        Check(client.Sends == 1, "uncertain referenced send is never replayed");
        client.Acknowledged = client.Sent!["request_id"]!.GetValue<string>();
        await model.RefreshAsync();
        Check(!model.UncertainSend && model.References.Length == 0 && model.Draft.Length == 0, "authoritative acknowledgement clears only the submitted context");

        model.AddReference(reference); client.Running = true;
        await model.RefreshAsync(); await model.QueueAsync();
        Check(client.Queued?["references"]?[0]?["id"]?.GetValue<string>() == "artifact-exact-id"
            && model.References.Length == 0, "queued follow-up preserves typed references");
        client.SupportsReferences = false; client.Running = false;
        await model.RefreshAsync();
        Check(!model.AddReference(reference), "old hosts do not expose unsupported reference writes");
        client.SupportsReferences = true; client.CancelSend = true; client.FailSend = false; client.Acknowledged = null;
        await model.OpenAsync("project-a", "session-c");
        model.AddReference(reference); model.Draft = "keep after cancellation";
        await model.SendAsync();
        Check(!model.Busy && model.UncertainSend && model.References.Length == 1 && model.Draft == "keep after cancellation",
            "cancelled send releases Busy and preserves uncertain typed context");
        Console.WriteLine("Native composer references: caret, identity, stale reads, drafts, sends and queue passed.");
    }

    private static async Task ReadingStateAsync()
    {
        var client = new Fake();
        var model = new WorkspaceConversationModel(new NativeConversationClient(client));
        await model.OpenAsync("p", "s");
        var source = model.Snapshot! with { Items = [new("user", "question", null, null, null, null),
            new("assistant", "quoted answer", null, null, null, null)], UserOffset = 4 };
        Check(!model.AddQuote(source with { ProjectId = "elsewhere" }, 1, "bad"), "cross-project source is rejected");
        Check(model.AddQuote(source, 1, "quoted answer") && model.CanSend, "quote-only draft is sendable");
        model.AddQuote(source, 1, "quoted answer");
        Check(model.Quotes.Length == 1, "same source selection is deduplicated");
        await model.OpenAsync("p", "another"); Check(model.Quotes.Length == 0, "quotes do not leak across sessions");
        await model.OpenAsync("p", "s"); Check(model.Quotes.Length == 1, "quotes survive navigation with their draft");
        client.FailSend = true; await model.SendAsync();
        Check(model.UncertainSend && model.Quotes.Length == 1 && client.Sent?["message"]?.GetValue<string>().Contains("第 5 轮") == true,
            "ambiguous send preserves quote and transports its original source");
        await model.SendAsync(); Check(client.Sends == 1, "quote sends are never automatically replayed");
        client.Acknowledged = client.Sent!["request_id"]!.GetValue<string>(); await model.RefreshAsync();
        Check(model.Quotes.Length == 0, "authoritative acknowledgement clears submitted quote");
        model.AddQuote(source, 1, "queue me"); client.Running = true; await model.RefreshAsync(); await model.QueueAsync();
        Check(model.Quotes.Length == 0 && client.Queued?["message"]?.GetValue<string>().Contains("> queue me") == true, "queue preserves quote source and content");
    }

    private static async Task DocumentReadingStateAsync()
    {
        var client = new Fake();
        var model = new WorkspaceConversationModel(new NativeConversationClient(client));
        await model.OpenAsync("p", "s");
        model.Draft = "keep my question";
        var selection = new NativeDocumentSelection("First line\nSecond line", 2, false);
        Check(!model.AddDocumentQuote("foreign", "s", "paper.pdf", selection)
            && !model.AddDocumentQuote("p", "old-session", "paper.pdf", selection), "late PDF callbacks cannot quote into a different scope");
        Check(!model.AddDocumentQuote("p", "s", "paper.pdf\nforged source", selection)
            && !model.AddDocumentQuote("p", "s", "paper.pdf", selection with { Location = new("pdf", Page: 0) }), "invalid source metadata is rejected");
        Check(model.AddDocumentQuote("p", "s", "literature/paper.pdf", selection), "PDF selection is added to the composer");
        model.AddDocumentQuote("p", "s", "literature/paper.pdf", selection);
        Check(model.Quotes.Length == 1 && model.Draft == "keep my question"
            && model.Quotes[0].Source.Contains("第 2 页") && !model.Quotes[0].Source.Contains("轮"), "PDF source is deduplicated without inventing a turn or replacing text");
        await model.OpenAsync("p", "another"); Check(model.Quotes.Length == 0, "PDF quotes stay in their owning draft");
        await model.OpenAsync("p", "s"); Check(model.Quotes.Length == 1, "PDF quotes survive navigation");
        client.FailSend = true; await model.SendAsync();
        var sent = client.Sent!["message"]!.GetValue<string>();
        Check(model.UncertainSend && model.Quotes.Length == 1 && sent.Contains("literature/paper.pdf · 第 2 页 · 项目 p")
            && sent.EndsWith("> First line\n> Second line\n\nkeep my question"), "send preserves file provenance, multiline quote, draft and uncertain recovery");
        Check(!model.AddDocumentQuote("p", "s", "paper.pdf", selection), "uncertain send cannot mutate staged quote payload");
        await model.SendAsync(); Check(client.Sends == 1, "PDF quote sends are not replayed");
        client.Acknowledged = client.Sent["request_id"]!.GetValue<string>(); await model.RefreshAsync();
        Check(model.Quotes.Length == 0, "authoritative acknowledgement clears PDF quotes");
        model.AddDocumentQuote("p", "s", "paper.pdf", selection); client.Running = true; await model.RefreshAsync(); await model.QueueAsync();
        Check(model.Quotes.Length == 0 && client.Queued?["message"]?.GetValue<string>().Contains("paper.pdf · 第 2 页") == true,
            "queued PDF excerpts keep their file and page");
        var cell = new NativeDocumentSelection("值：84\n公式：=A1*2", new NativeDocumentLocation("xlsx", Sheet: "实验数据", Cells: "B2"), false);
        var sourceExcerpt = NativeDocumentSelection.FromSource(">seq\nACGT\n", 5, 4, false)!;
        client.Running = false; await model.RefreshAsync(); // Complete the previous legacy queued turn.
        client.Running = true; await model.RefreshAsync();
        model.Draft = "compare these";
        Check(model.AddDocumentQuote("p", "s", "counts.xlsx", cell)
            && model.AddDocumentQuote("p", "s", "aligned.fa", sourceExcerpt), "Office and scientific quotes enter the same session draft");
        model.AddDocumentQuote("p", "s", "counts.xlsx", cell);
        Check(model.Quotes.Length == 2 && model.Draft == "compare these", "Office quotes deduplicate without replacing composer text");
        await model.OpenAsync("p", "another"); Check(model.Quotes.Length == 0, "rich document quotes never cross sessions");
        await model.OpenAsync("p", "s"); Check(model.Quotes.Length == 2, "rich document quote locators survive navigation");
        await model.QueueAsync();
        var queued = client.Queued!["message"]!.GetValue<string>();
        Check(queued.Contains("counts.xlsx · 工作表 实验数据 · B2") && queued.Contains("aligned.fa · 源文本第 2 行")
            && queued.Contains("> 公式：=A1*2") && queued.EndsWith("compare these"), "queued content retains sheet/cell, formula and scientific line source");
    }

    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static JsonNode Catalog(string session, string id) => JsonSerializer.SerializeToNode(
        new NativeReferenceCatalog(session, [new(new("artifact", Id: id), id, "")]), ConversationSnapshot.JsonOptions)!;

    private sealed class Fake : INativeSettingsClient
    {
        public TaskCompletionSource<JsonNode?>? Pending;
        public TaskCompletionSource Entered = new();
        public bool WrongIdentity, FailSend, CancelSend, Running;
        public bool SupportsReferences = true;
        public string? LastProject, LastSession, Acknowledged;
        public JsonObject? Sent, Queued;
        public int Sends;
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            cancellationToken.ThrowIfCancellationRequested();
            LastProject = projectId; LastSession = args["session_id"]?.GetValue<string>();
            switch (command)
            {
                case "native_conversation_references":
                    Entered.TrySetResult();
                    return Pending?.Task ?? Task.FromResult<JsonNode?>(Catalog(WrongIdentity ? "wrong" : LastSession!, args["query"]!.GetValue<string>()));
                case "native_conversation_snapshot":
                    return Task.FromResult(JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId,
                        "host", ++sequence, projectId!, LastSession!, [], null, Running, false, false, "model", Acknowledged, null, [],
                        ComposerReferences: SupportsReferences), ConversationSnapshot.JsonOptions));
                case "native_conversation_send":
                    Sends++; Sent = (JsonObject)args.DeepClone();
                    if (CancelSend) throw new OperationCanceledException();
                    if (FailSend) throw new IOException("acknowledgement lost");
                    return Task.FromResult<JsonNode?>(new JsonObject());
                case "native_conversation_enqueue":
                    Queued = (JsonObject)args.DeepClone();
                    return Task.FromResult<JsonNode?>(new JsonObject { ["queued"] = true });
                case "list_models": return Task.FromResult<JsonNode?>(new JsonArray());
                default: return Task.FromResult<JsonNode?>(null);
            }
        }
    }
}
