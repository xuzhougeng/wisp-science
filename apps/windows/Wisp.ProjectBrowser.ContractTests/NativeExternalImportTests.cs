using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeExternalImportTests
{
    public static async Task RunAsync(string fixture)
    {
        void Check(bool ok, string name) { if (!ok) throw new Exception(name); Console.WriteLine("PASS external import: " + name); }
        var json = JsonNode.Parse(File.ReadAllText(fixture))!;
        var sources = NativeExternalImportClient.DecodeSources(json["sources"], "research-1");
        var list = NativeExternalImportClient.DecodeList(json["list"], "research-1", "codex", "wsl:Ubuntu");
        var reviewed = NativeExternalImportClient.DecodePreview(json["preview"], list.ProjectId, list.Provider, list.ContextId, list.Items[0]);
        Check(sources.Sources.Length == 3 && NativeExternalImportClient.DecodeResult(json["result"], reviewed).FrameId == "imported-1", "shared fixture retains local WSL SSH sources and exact import destination");
        foreach (var field in new[] { "project_id", "provider", "context_id", "path", "source_session_id", "sha256" })
        {
            var bad = json["preview"]!.DeepClone(); bad[field] = "wrong";
            try { NativeExternalImportClient.DecodePreview(bad, list.ProjectId, list.Provider, list.ContextId, list.Items[0]); throw new Exception("accepted wrong " + field); }
            catch (InvalidDataException) { }
        }
        foreach (var field in new[] { "project_id", "provider", "context_id", "path", "source_session_id", "status" })
        {
            var bad = json["result"]!.DeepClone(); bad[field] = "wrong";
            try { NativeExternalImportClient.DecodeResult(bad, reviewed); throw new Exception("accepted wrong result " + field); }
            catch (InvalidDataException) { }
        }
        var duplicate = json["list"]!.DeepClone(); duplicate["items"]!.AsArray().Add(duplicate["items"]![0]!.DeepClone());
        try { NativeExternalImportClient.DecodeList(duplicate, list.ProjectId, list.Provider, list.ContextId); throw new Exception("accepted duplicate source path"); }
        catch (InvalidDataException) { }
        var fake = new Fake { Count = 53 };
        using var model = new WorkspaceExternalImportModel(fake, "research-1"); await model.InitializeAsync();
        Check(model.Sources.Length == 3 && fake.Refreshes.SequenceEqual(new[] { false }) && model.PageItems.Length == 25 && model.PageCount == 3, "initial cached list paginates at 25 without forced scan");
        model.SetPage(2); Check(model.PageItems.Length == 3, "last page retains remaining rows");
        model.SetQuery("session-52"); Check(model.Page == 0 && model.Filtered.Single().SessionId == "session-52", "filter covers session IDs and resets pagination");
        model.SetQuery("/work/52"); Check(model.Filtered.Length == 1, "directory query filters imported source metadata");
        model.SetQuery(""); await model.LoadAsync(true); Check(fake.Refreshes.Last(), "explicit refresh requests a source rescan");
        fake.PendingList = new(); var lateList = model.LoadAsync(false); var pendingList = fake.PendingList;
        fake.PendingList = null; model.Select("another", "claude", "wsl:Ubuntu"); await model.LoadAsync(false);
        pendingList.SetResult(Fake.List("research-1", "codex", "local", 1)); await lateList;
        Check(model.Provider == "claude" && model.Items.Length == 53 && model.Items[0].Title.StartsWith("claude"), "old provider/project list cannot replace a newer destination");
        fake.PendingPreview = new(); var oldItem = model.Items[0]; var latePreview = model.PreviewAsync(oldItem); var pendingPreview = fake.PendingPreview;
        fake.PendingPreview = null; model.Select("another", "codex", "ssh:analysis"); await model.LoadAsync(false);
        pendingPreview.SetResult(Fake.Preview("another", "claude", "wsl:Ubuntu", oldItem)); await latePreview;
        Check(model.Preview == null && !model.Previewing, "source switch rejects a delayed preview with the same file name");
        foreach (var changePage in new[] { false, true })
        {
            fake.PendingPreview = new(); oldItem = model.Items[0]; latePreview = model.PreviewAsync(oldItem); pendingPreview = fake.PendingPreview;
            if (changePage) model.SetPage(1); else model.SetQuery("session-0");
            pendingPreview.SetResult(Fake.Preview(model.Project, model.Provider, model.Context, oldItem)); await latePreview;
            Check(model.Preview == null && !model.Previewing && !model.CanImport,
                changePage ? "page change rejects an outstanding preview" : "filter change rejects an outstanding preview");
            fake.PendingPreview = null; model.SetQuery("");
        }
        var pagedFake = new Fake { Count = 53 }; using var paged = new WorkspaceExternalImportModel(pagedFake, "research-1");
        await paged.LoadAsync(false); paged.SetQuery("codex"); paged.SetPage(2); await paged.ImportFilteredAsync();
        Check(pagedFake.Writes.Count == 53 && paged.Done == 53 && paged.Imported == 53 && !paged.CanImportFiltered,
            "bulk import includes filtered rows across all pages rather than only the displayed page");
        fake.Count = 3; await model.LoadAsync(false); await model.PreviewAsync(model.Items[0]);
        fake.PendingWrite = new(); var one = model.ImportPreviewAsync();
        model.Select("foreign", "claude", "local"); model.SetQuery("foreign"); await model.ImportFilteredAsync();
        Check(fake.Writes.Count == 1 && model.Project == "another" && model.Query == "", "in-flight write freezes selection and prevents duplicate batch dispatch");
        fake.PendingWrite.SetException(new IOException("response lost")); await one; fake.PendingWrite = null;
        await model.LoadAsync(false); await model.PreviewAsync(model.Items[0]); await model.ImportPreviewAsync(); await model.ImportFilteredAsync();
        Check(model.Uncertain && fake.Writes.Count == 1 && !model.CanImport && !model.CanImportFiltered, "ambiguous reply stops all writes including after refresh and preview");
        var batchFake = new Fake { Count = 3, FailPreviewPath = "/source/0.jsonl" };
        using var batch = new WorkspaceExternalImportModel(batchFake, "research-1"); await batch.LoadAsync(false); await batch.ImportFilteredAsync();
        Check(batch.Done == 3 && batch.Failed == 1 && batch.Imported == 2 && batchFake.Writes.Count == 2 && batch.ItemErrors.Count == 1,
            "batch continues after a read-only source failure and reports item-level totals");
        var stopFake = new Fake { Count = 3, PendingWrite = new() };
        using var stopping = new WorkspaceExternalImportModel(stopFake, "research-1"); await stopping.LoadAsync(false); var stoppingTask = stopping.ImportFilteredAsync();
        stopping.StopAfterCurrent(); stopFake.PendingWrite.SetResult(Fake.Result(stopFake.Writes.Single())); await stoppingTask;
        Check(stopping.Done == 1 && stopping.Total == 3 && stopping.Imported == 1 && stopFake.Writes.Count == 1, "stop preserves current confirmation and does not start the next import");
        var closeFake = new Fake { Count = 3, PendingPreview = new() };
        var closing = new WorkspaceExternalImportModel(closeFake, "research-1"); await closing.LoadAsync(false); var item = closing.Items[0]; var closeTask = closing.ImportFilteredAsync(); closing.Dispose();
        closeFake.PendingPreview.SetResult(Fake.Preview("research-1", "codex", "local", item)); await closeTask;
        Check(closeFake.Writes.Count == 0, "closing during batch preview prevents its subsequent write");
        closeFake = new Fake { Count = 3, PendingWrite = new() }; closing = new(closeFake, "research-1"); await closing.LoadAsync(false); closeTask = closing.ImportFilteredAsync(); closing.Dispose();
        closeFake.PendingWrite.SetResult(Fake.Result(closeFake.Writes.Single())); await closeTask;
        Check(closeFake.Writes.Count == 1 && closing.Results.Count == 0, "closed batch ignores its write reply and cannot continue or navigate");
        var outcomes = new Fake { Count = 3, OutcomeByIndex = true }; using var done = new WorkspaceExternalImportModel(outcomes, "research-1"); await done.LoadAsync(false); await done.ImportFilteredAsync();
        Check(done.Imported == 1 && done.Updated == 1 && done.Skipped == 1 && done.Done == 3 && !done.CanImportFiltered, "mixed confirmed outcomes update rows without rescanning or replay");
    }
    private sealed class Fake : INativeExternalImportClient
    {
        public int Count = 3;
        public bool OutcomeByIndex;
        public string? FailPreviewPath;
        public TaskCompletionSource<ExternalImportList>? PendingList;
        public TaskCompletionSource<ExternalImportPreview>? PendingPreview;
        public TaskCompletionSource<ExternalImportResult>? PendingWrite;
        public List<bool> Refreshes = [];
        public List<ExternalImportPreview> Writes = [];
        public static ExternalImportList List(string project, string provider, string context, int count) => new(NativeExternalImportClient.Schema, project, provider, context,
            Enumerable.Range(0, count).Select(i => new ExternalImportItem($"/source/{i}.jsonl", $"session-{i}", $"{provider} synthetic {i}", $"/work/{i}", 2, 1, "new")).ToArray());
        public static ExternalImportPreview Preview(string project, string provider, string context, ExternalImportItem item) => new(NativeExternalImportClient.Schema,
            project, provider, context, item.Path, item.SessionId, new string('a', 64), 2, [new("user", "Synthetic question"), new("assistant", "Synthetic answer")], null);
        public static ExternalImportResult Result(ExternalImportPreview reviewed, string status = "imported") => new(NativeExternalImportClient.Schema, reviewed.ProjectId,
            reviewed.Provider, reviewed.ContextId, reviewed.Path, reviewed.SourceSessionId, "frame-" + reviewed.SourceSessionId, status, reviewed.MessageCount);
        public Task<ExternalImportSources> SourcesAsync(string project, CancellationToken cancellationToken = default) => Task.FromResult(new ExternalImportSources(NativeExternalImportClient.Schema,
            project, [new("local", "Local", "local"), new("wsl:Ubuntu", "Ubuntu", "wsl"), new("ssh:analysis", "Analysis", "ssh")]));
        public Task<ExternalImportList> ListAsync(string project, string provider, string context, bool refresh, CancellationToken cancellationToken = default)
        { Refreshes.Add(refresh); return PendingList?.Task ?? Task.FromResult(List(project, provider, context, Count)); }
        public Task<ExternalImportPreview> PreviewAsync(string project, string provider, string context, ExternalImportItem item, CancellationToken cancellationToken = default) => item.Path == FailPreviewPath
            ? Task.FromException<ExternalImportPreview>(new IOException("Source unavailable")) : PendingPreview?.Task ?? Task.FromResult(Preview(project, provider, context, item));
        public Task<ExternalImportResult> ImportAsync(ExternalImportPreview reviewed)
        { Writes.Add(reviewed); return PendingWrite?.Task ?? Task.FromResult(Result(reviewed, OutcomeByIndex ? new[] { "imported", "updated", "skipped" }[Writes.Count - 1] : "imported")); }
    }
}
