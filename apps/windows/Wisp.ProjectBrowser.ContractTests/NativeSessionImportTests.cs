using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSessionImportTests
{
    public static async Task RunAsync(string fixture)
    {
        void Check(bool ok, string name) { if (!ok) throw new Exception(name); Console.WriteLine("PASS session archive import: " + name); }
        var json = JsonNode.Parse(File.ReadAllText(fixture))!;
        var reviewed = NativeSessionImportClient.DecodePreview(json["preview"], "research-1", @"C:\exports\session.zip");
        Check(NativeSessionImportClient.DecodeResult(json["result"], reviewed).FrameId == "imported-1", "shared Rust fixture preserves reviewed archive and exact destination");
        foreach (var mutate in new Action<JsonNode>[] {
            n => n["project_id"] = "foreign", n => n["archive_path"] = "other.zip", n => n["sha256"] = "bad",
            n => n["state"] = "updatable", n => n["messages"]![0]!["role"] = "unknown" })
        {
            var bad = json["preview"]!.DeepClone(); mutate(bad);
            try { NativeSessionImportClient.DecodePreview(bad, reviewed.ProjectId, reviewed.ArchivePath); throw new Exception("accepted invalid preview"); }
            catch (InvalidDataException) { }
        }
        foreach (var mutate in new Action<JsonNode>[] {
            n => n["project_id"] = "foreign", n => n["source_session_id"] = "foreign", n => n["message_count"] = 3,
            n => n["status"] = "unknown", n => n["artifact_count"] = 2 })
        {
            var bad = json["result"]!.DeepClone(); mutate(bad);
            try { NativeSessionImportClient.DecodeResult(bad, reviewed); throw new Exception("accepted invalid result"); }
            catch (InvalidDataException) { }
        }
        var client = new Fake(reviewed);
        using var model = new WorkspaceArchiveImportModel(client, "research-1");
        model.Select("research-1", reviewed.ArchivePath);
        client.PendingPreview = new(); var stale = model.PreviewAsync(); var old = client.PendingPreview;
        client.PendingPreview = null; model.Select("another", reviewed.ArchivePath); await model.PreviewAsync();
        old.SetResult(reviewed); await stale;
        Check(model.Preview?.ProjectId == "another" && model.CanImport, "late original-project preview cannot replace the selected destination");
        client.PendingImport = new(); var writing = model.ImportAsync();
        model.Select("foreign", @"C:\different.zip"); await model.ImportAsync();
        Check(client.Writes == 1 && model.Project == "another" && model.Path == reviewed.ArchivePath, "busy write freezes destination and cannot be submitted twice");
        client.PendingImport.SetException(new IOException("response lost")); await writing;
        await model.ImportAsync(); await model.PreviewAsync(); await model.ImportAsync();
        Check(client.Writes == 1 && model.Uncertain && !model.CanImport && model.Path == reviewed.ArchivePath,
            "lost import response keeps selection and cannot be replayed even after a fresh preview");
        client.PendingImport = null;
        using var success = new WorkspaceArchiveImportModel(client, "research-1");
        success.Select("research-1", reviewed.ArchivePath); await success.PreviewAsync(); await success.ImportAsync();
        Check(success.Result?.ProjectId == "research-1" && success.Result.FrameId == "imported-1" && !success.CanImport,
            "confirmed result exposes exact session and prevents duplicate import");
        var closed = new WorkspaceArchiveImportModel(client, "research-1"); closed.Select("research-1", reviewed.ArchivePath);
        client.PendingPreview = new(); var closing = closed.PreviewAsync(); closed.Dispose();
        client.PendingPreview.SetResult(reviewed); await closing;
        Check(closed.Preview == null && !closed.CanImport && !closed.Reading, "Escape discards late previews without enabling an import");
        client.PendingPreview = null; client.FailPreview = true;
        using var failed = new WorkspaceArchiveImportModel(client, "research-1"); failed.Select("research-1", reviewed.ArchivePath); await failed.PreviewAsync();
        Check(failed.Error != null && failed.Preview == null && failed.Path == reviewed.ArchivePath && !failed.CanImport, "corrupt/unreadable archive retains selection and cannot be imported");
        client.FailPreview = false;
        var closingWrite = new WorkspaceArchiveImportModel(client, "research-1"); closingWrite.Select("research-1", reviewed.ArchivePath); await closingWrite.PreviewAsync();
        client.PendingImport = new(); var pendingWrite = closingWrite.ImportAsync(); closingWrite.Dispose();
        client.PendingImport.SetResult(Fake.Result(reviewed)); await pendingWrite;
        Check(closingWrite.Result == null, "closing during import never navigates on its eventual response");
    }
    private sealed class Fake(NativeArchivePreview sample) : INativeSessionImportClient
    {
        public TaskCompletionSource<NativeArchivePreview>? PendingPreview;
        public TaskCompletionSource<NativeArchiveImportResult>? PendingImport;
        public bool FailPreview;
        public int Writes;
        public Task<NativeArchivePreview> PreviewAsync(string project, string path, CancellationToken cancellationToken = default) => FailPreview
            ? Task.FromException<NativeArchivePreview>(new IOException("Unreadable archive"))
            : PendingPreview?.Task ?? Task.FromResult(sample with { ProjectId = project, ArchivePath = path });
        public static NativeArchiveImportResult Result(NativeArchivePreview preview) => new(NativeSessionImportClient.Schema, preview.ProjectId, preview.SourceSessionId,
            "imported-1", "imported", preview.MessageCount, 1, []);
        public Task<NativeArchiveImportResult> ImportAsync(NativeArchivePreview reviewed, CancellationToken cancellationToken = default)
        { Writes++; return PendingImport?.Task ?? Task.FromResult(Result(reviewed)); }
    }
}
