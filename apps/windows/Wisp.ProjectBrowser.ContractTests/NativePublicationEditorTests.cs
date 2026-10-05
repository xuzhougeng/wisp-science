using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativePublicationEditorTests
{
    private static void Check(bool value, string label) { if (!value) throw new Exception(label); Console.WriteLine("PASS publication editor: " + label); }
    public static async Task RunAsync()
    {
        var fake = new Fake(); var client = new NativePublicationClient(fake);
        using var model = new WorkspacePublicationModel(client, "p");
        Check(await model.LoadAsync() && model.Editable && model.Workspace!.Bindings!.Single().SourceId == "exact-v1", "full evidence contract retains exact source and editable revision");
        model.SelectItem("item"); model.ItemDraft = model.ItemDraft with { Content = "modified\n中文😀" };
        fake.Fail = true; var before = fake.Writes;
        Check(!await model.SaveItemAsync() && model.MutationUncertain && model.ItemDraft.Content == "modified\n中文😀", "uncertain save retains draft and blocks mutations");
        await model.SaveItemAsync(); Check(fake.Writes == before + 1, "repeated save does not replay an uncertain write");
        fake.Fail = false; await model.LoadAsync();
        Check(model.MutationUncertain && model.Reconciled, "successful reconciliation does not silently authorize another write");
        model.AcknowledgeUncertain(); Check(model.Editable, "explicit reconciliation restores editing");
        model.SelectItem(null); model.ItemDraft = model.ItemDraft with { Title = "new draft" };
        model.SelectItem("item"); Check(model.ItemDraft.Content == "modified\n中文😀", "switching items preserves unsaved editor text");
        model.SelectItem(null); Check(model.ItemDraft.Title == "new draft", "new-item draft survives switching to an existing item"); model.SelectItem("item");
        Check(await model.SaveItemAsync() && fake.LastOperation?["content"]?.GetValue<string>() == "modified\n中文😀", "confirmed save sends multiline draft to exact revision");
        model.ItemDraft = model.ItemDraft with { Content = "unsaved revision one" };
        Check(await model.SelectWorkspaceAsync("pub", "r2") && model.ItemDraft.Content == "", "new revision starts its own editor draft");
        await model.SelectWorkspaceAsync("pub", "r");
        Check(model.ItemDraft.Content == "unsaved revision one", "returning to a revision restores its unsaved editor draft");
        await model.LoadSourcesAsync("messages", "Chinese", 0);
        var source = model.Sources.Sources.Single(); model.SelectSource(source);
        var locator = JsonNode.Parse(WorkspacePublicationModel.MessageLocator(source, 1, 3))!;
        Check(locator["byte_start"]!.GetValue<int>() == 1 && locator["byte_end"]!.GetValue<int>() == 8
            && locator["message_content_sha256"]!.GetValue<string>() == "digest", "Unicode selection maps UTF16 to exact UTF8 bytes with content digest");
        try { WorkspacePublicationModel.MessageLocator(source, 3, 1); throw new Exception("surrogate split accepted"); } catch (InvalidDataException) { }
        Check(await model.BindSourceAsync("item", "supports claim", null, "selected", "private", 1, 3)
            && JsonNode.Parse(fake.LastOperation!["source_id"]!.GetValue<string>())!["byte_end"]!.GetValue<int>() == 8,
            "binding submits the exact selected message locator");
        Check(NativePublicationEvidence.SelectionChoices.Select(v => v.Id).SequenceEqual(new[] { "candidate", "selected", "rejected" }), "UI selection options use host evidence states");
        foreach (var kind in new[] { "execution_log", "code_cell", "external_resource", "tool_call", "message_span" }) {
            model.SelectPreciseSource(kind, " frame ", "7", "1", "8", " call ", " exact ");
            Check(await model.BindSourceAsync("item", "precise evidence", null, "rejected", "private")
                && fake.LastOperation?["source_kind"]?.GetValue<string>() == kind, "precise source binds without list membership: " + kind);
            var id = fake.LastOperation!["source_id"]!.GetValue<string>();
            if (kind == "message_span") Check(JsonNode.Parse(id)!["byte_end"]!.GetValue<int>() == 8, "manual message locator preserves byte bounds");
            else if (kind == "tool_call") Check(JsonNode.Parse(id)!["tool_call_id"]!.GetValue<string>() == "call", "tool locator preserves exact call ID");
            else Check(id == "exact", "registered source uses exact trimmed ID");
        }
        foreach (var invalid in new[] { ("message_span", "0", "1", "8", "call", "id"), ("message_span", "7", "8", "1", "call", "id"), ("tool_call", "7", "0", "8", "", "id"), ("external_resource", "7", "0", "8", "call", "") }) {
            var currentSource = model.Source;
            try { model.SelectPreciseSource(invalid.Item1, "frame", invalid.Item2, invalid.Item3, invalid.Item4, invalid.Item5, invalid.Item6); throw new Exception("invalid precise source accepted"); } catch (InvalidDataException) { }
            Check(ReferenceEquals(currentSource, model.Source), "invalid precise input retains reviewed source");
        }
        before = fake.Writes; model.ClearSource();
        Check(model.Source == null && !await model.BindSourceAsync(null, "stale", null, "selected", "private") && fake.Writes == before,
            "returning to source selection prevents binding the dismissed precise source");
        fake.SourcePending = new(TaskCreationOptions.RunContinuationsAsynchronously); var pendingSource = fake.SourcePending;
        var old = model.LoadSourcesAsync("messages", "old", 0);
        await model.LoadSourcesAsync("files", "new", 0);
        pendingSource.SetResult(new JsonObject { ["sources"] = new JsonArray(), ["has_more"] = true }); await old;
        Check(model.Sources.Sources.Single().Id == "exact-v1" && !model.Sources.HasMore, "late source search cannot replace the current filter result");
        fake.SourcePending = new(TaskCreationOptions.RunContinuationsAsynchronously); pendingSource = fake.SourcePending;
        old = model.LoadSourcesAsync("messages", "old revision", 0); await model.SelectWorkspaceAsync("pub", "r2");
        pendingSource.SetResult(new JsonObject { ["sources"] = new JsonArray(), ["has_more"] = true }); await old;
        Check(model.Sources.Sources.Length == 0 && !model.Sources.HasMore, "revision navigation rejects an old source page");
        fake.Foreign = true; var selected = model.Workspace;
        Check(!await model.LoadAsync() && ReferenceEquals(selected, model.Workspace), "foreign workspace reply retains selected revision"); fake.Foreign = false;
        fake.Frozen = true; await model.LoadAsync(); before = fake.Writes;
        Check(!model.Editable && !await model.SaveItemAsync() && fake.Writes == before, "frozen version cannot write item drafts");
        Check(!model.CanBuildCapsule && !await model.MutateAsync(new() { ["action"] = "build_capsule", ["destination"] = "fixture.zip" }) && fake.Writes == before,
            "export requires a confirmed frozen manifest");
        fake.ReadPending = new(TaskCreationOptions.RunContinuationsAsynchronously); var pendingRead = fake.ReadPending;
        var closing = model.LoadAsync(); model.Dispose(); pendingRead.SetResult(Fake.Page("r")); await closing;
        Check(model.RevisionId == "r2", "closed page rejects a delayed workspace response");
        using var cloneModel = new WorkspacePublicationModel(new NativePublicationClient(new Fake()), "p");
        await cloneModel.LoadAsync();
        Check(!await cloneModel.MutateAsync(new() { ["action"] = "clone_revision", ["label"] = "v2" }) && cloneModel.MutationUncertain && cloneModel.RevisionId == "r",
            "clone reply returning the original revision is rejected without replay");
        var cloning = new Fake { Clone = true };
        using var validClone = new WorkspacePublicationModel(new NativePublicationClient(cloning), "p");
        await validClone.LoadAsync(); validClone.SelectItem("item"); validClone.ItemDraft = validClone.ItemDraft with { Content = "retain original draft" };
        Check(await validClone.MutateAsync(new() { ["action"] = "clone_revision", ["label"] = "v2" }) && validClone.RevisionId == "r2", "confirmed clone selects its new child revision");
        await validClone.SelectWorkspaceAsync("pub", "r");
        Check(validClone.ItemDraft.Content == "retain original draft", "cloning retains the parent revision draft");
        var uncertainCreate = new Fake();
        using var draftModel = new WorkspacePublicationModel(new NativePublicationClient(uncertainCreate), "p");
        await draftModel.LoadAsync(); draftModel.SelectItem(null);
        before = uncertainCreate.Writes;
        Check(!await draftModel.SaveItemAsync() && draftModel.Error == "请填写条目标题。" && uncertainCreate.Writes == before,
            "missing title gives feedback without dispatch or uncertain state");
        draftModel.ItemDraft = draftModel.ItemDraft with { Title = "new evidence item", Ordinal = -1 };
        Check(!await draftModel.SaveItemAsync() && draftModel.Error == "排序必须是非负整数。" && !draftModel.MutationUncertain,
            "invalid ordering gives feedback without blocking subsequent edits");
        draftModel.ItemDraft = draftModel.ItemDraft with { Ordinal = 1, Content = "pending creation" };
        draftModel.SelectItem("item"); draftModel.SelectItem(null); // Exercise cached unnamed draft.
        uncertainCreate.Fail = true;
        Check(!await draftModel.SaveItemAsync(), "new-item lost reply remains unconfirmed");
        var pendingId = draftModel.ItemDraft.Id;
        uncertainCreate.Fail = false; await draftModel.LoadAsync();
        Check(draftModel.SelectedItem == pendingId && draftModel.ItemDraft.Id == pendingId && draftModel.Reconciled,
            "reconciliation with no saved row preserves the pending item identity");
        before = uncertainCreate.Writes; draftModel.AcknowledgeUncertain();
        draftModel.SelectItem("item"); draftModel.SelectItem(null);
        Check(draftModel.ItemDraft.Content == "pending creation" && draftModel.ItemDraft.Id == pendingId,
            "New item reopens the pending creation after selecting another item");
        Check(await draftModel.SaveItemAsync() && uncertainCreate.LastOperation?["id"]?.GetValue<string>() == pendingId
            && uncertainCreate.Writes == before + 1, "explicit save after reconciliation reuses the reviewed item identity");
        draftModel.SelectItem(null);
        Check(draftModel.ItemDraft.Id == null && draftModel.ItemDraft.Title.Length == 0,
            "new-item entry cannot resurrect the cached pre-save draft after reconciliation");
        draftModel.ItemDraft = draftModel.ItemDraft with { Title = "independent unnamed draft" };
        draftModel.SelectItem("item"); await draftModel.SaveItemAsync(); draftModel.SelectItem(null);
        Check(draftModel.ItemDraft.Title == "independent unnamed draft",
            "saving an existing item retains the separate unnamed draft");
        var committedCreate = new Fake { PersistItems = true };
        using var committedModel = new WorkspacePublicationModel(new NativePublicationClient(committedCreate), "p");
        await committedModel.LoadAsync(); committedModel.SelectItem(null);
        committedModel.ItemDraft = committedModel.ItemDraft with { Title = "committed before lost reply", Content = "original creation" };
        committedCreate.Fail = true; await committedModel.SaveItemAsync();
        var committedId = committedModel.ItemDraft.Id;
        committedModel.ItemDraft = committedModel.ItemDraft with { Content = "local unsaved correction" };
        committedModel.SelectItem("item");
        committedCreate.Fail = false; await committedModel.LoadAsync(); committedModel.AcknowledgeUncertain();
        committedModel.SelectItem(null);
        Check(committedModel.ItemDraft.Id == null && committedModel.ItemDraft.Title == "" && committedCreate.Writes == 1,
            "confirmed committed creation retires its New item alias without another write");
        committedModel.SelectItem(committedId);
        Check(committedModel.ItemDraft.Content == "local unsaved correction",
            "confirmed creation retains local corrections under the persisted item identity");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool Fail, Foreign, Frozen, Clone, PersistItems;
        private readonly Dictionary<(string Revision, string Id), JsonObject> savedItems = [];
        public int Writes;
        public JsonObject? LastOperation;
        public TaskCompletionSource<JsonNode?>? SourcePending, ReadPending;
        public static JsonObject Page(string revision) => new() {
            ["publications"] = new JsonArray(new JsonObject { ["id"]="pub",["project_id"]="p",["title"]="Paper",["description"]="" }),
            ["publication"] = new JsonObject { ["id"]="pub",["project_id"]="p",["title"]="Paper",["description"]="" },
            ["revision"] = new JsonObject { ["id"]=revision,["publication_id"]="pub",["label"]=revision,["state"]="draft" },
            ["revisions"] = new JsonArray(new JsonObject { ["id"]="r",["publication_id"]="pub",["label"]="v1",["state"]="draft" },new JsonObject { ["id"]="r2",["publication_id"]="pub",["label"]="v2",["state"]="draft" }),
            ["items"] = new JsonArray(new JsonObject { ["id"]="item",["revision_id"]=revision,["kind"]="claim",["title"]="Claim",["ordinal"]=0,["content"]="original" }),
            ["bindings"] = new JsonArray(new JsonObject { ["id"]="binding",["revision_id"]=revision,["item_id"]="item",["source_kind"]="artifact_version",["source_id"]="exact-v1",["purpose"]="Evidence",["selection_state"]="selected",["review_state"]="unreviewed",["reproduction_state"]="unverified",["visibility"]="private",["source_snapshot_json"]="{}" })
        };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (projectId != "p") throw new Exception("Wrong project");
            if (command == "native_publication_sources") {
                if (SourcePending is { } pending) { SourcePending = null; return pending.Task; }
                var message = args["kind"]!.GetValue<string>() == "messages";
                return Task.FromResult<JsonNode?>(new JsonObject { ["sources"] = new JsonArray(new JsonObject {
                    ["id"] = message ? "message" : "exact-v1", ["kind"] = message ? "message_span" : "artifact_version", ["title"]="Source", ["detail"]="details",
                    ["text"] = message ? "A中😀B" : null, ["frame_id"]="frame", ["message_seq"]=7, ["text_sha256"]="digest"
                }), ["has_more"] = false });
            }
            var page = Page(args["revision_id"]?.GetValue<string>() ?? "r");
            if (Foreign) page["publication"]!["project_id"] = "foreign";
            if (Frozen) page["revision"]!["state"] = "frozen";
            if (command == "native_publication_mutate") {
                Writes++; LastOperation = (JsonObject)args["operation"]!.DeepClone();
                if (PersistItems && LastOperation["action"]?.GetValue<string>() == "save_item") {
                    var row = (JsonObject)LastOperation.DeepClone(); row.Remove("action");
                    var revision = args["revision_id"]!.GetValue<string>(); row["revision_id"] = revision;
                    savedItems[(revision, row["id"]!.GetValue<string>())] = row;
                }
                if (Fail) throw new IOException("lost reply");
                if (Clone && LastOperation["action"]?.GetValue<string>() == "clone_revision") { page = Page("r2"); page["revision"]!["parent_revision_id"] = args["revision_id"]!.GetValue<string>(); }
                return Task.FromResult<JsonNode?>(new JsonObject { ["workspace"] = page, ["readiness"] = null });
            }
            if (ReadPending is { } read) { ReadPending = null; return read.Task; }
            foreach (var entry in savedItems.Where(entry => entry.Key.Revision == page["revision"]!["id"]!.GetValue<string>()))
                page["items"]!.AsArray().Add(entry.Value.DeepClone());
            return Task.FromResult<JsonNode?>(page);
        }
    }
}
