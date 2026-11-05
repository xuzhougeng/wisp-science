using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeStoragePresentationTests
{
    public static async Task RunAsync()
    {
        var json = JsonNode.Parse("""
            {"data_dir":"C:\\shared","total_bytes":4096,"entries":[
              {"key":"database","bytes":1024},{"key":"python","bytes":0},
              {"key":"other","bytes":3072}],"projects":[
              {"id":"a","name":"Duplicate","path":"D:\\a","bytes":1024},
              {"id":"b","name":"Duplicate","path":"D:\\b","bytes":0}]}
            """)!;
        var original = json.ToJsonString(); var storage = NativeStoragePresentation.From(json);
        var all = storage.Select(null);
        Check(all.ProjectId == null && all.Path == "C:\\shared" && all.TotalBytes == 4096 && all.Entries.Count == 3,
            "all projects retains the host total, entry order and zero-byte categories");
        var selected = storage.Select("a");
        Check(selected.Path == "D:\\a" && selected.TotalBytes == 1024 && selected.Entries.SequenceEqual(new[] { new NativeStorageEntry("workspace", 1024) }),
            "project view excludes shared database/environment bytes and selects by id rather than duplicate name");
        var empty = storage.Select("b");
        Check(empty.ProjectId == "b" && empty.TotalBytes == 0 && empty.Entries.Single().Bytes == 0,
            "an empty project remains selected with one zero-byte workspace legend entry");
        Check(storage.Select("removed") == all && storage.Select(null) == all && json.ToJsonString() == original,
            "stale selection falls back to the shared overview without mutating the host snapshot");
        Check(NativeStoragePresentation.Fraction(0, 0) == 0 && NativeStoragePresentation.Fraction(1024, 4096) == .25
            && NativeStoragePresentation.Fraction(ulong.MaxValue, ulong.MaxValue) == 1,
            "chart proportions handle zero totals and unsigned host sizes without overflow");
        Check(NativeStoragePresentation.FormatBytes(1023) == "1023 B" && NativeStoragePresentation.FormatBytes(1024) == "1.0 KB"
            && NativeStoragePresentation.FormatBytes(1048576) == "1.0 MB" && NativeStoragePresentation.FormatBytes(1073741824) == "1.0 GB",
            "byte formatting uses the WebView binary thresholds");
        Check(new NativeStorageEntry("future-category", 2).LabelKey == "settings.storage.other"
            && new NativeStorageEntry("future-category", 2).ColorToken == "border-strong",
            "unknown host categories retain their bytes with the Other label and neutral color");
        Check(NativeStoragePresentation.Columns(567, 3) == 1 && NativeStoragePresentation.Columns(568, 3) == 2
            && NativeStoragePresentation.Columns(856, 3) == 3 && NativeStoragePresentation.Columns(2000, 2) == 2
            && NativeStoragePresentation.Columns(double.NaN, 3) == 1,
            "project tiles follow auto-fit 280px columns with 8px gaps, including unmeasured and narrow windows");

        var client = new ScanClient(); var browser = new NativeStorageBrowser();
        var first = browser.RefreshAsync(client, "scope", default);
        client.Pending.SetResult(json); await first; browser.Select("a");
        var before = browser.Snapshot;
        var next = browser.RefreshAsync(client, "scope", default);
        Check(browser.Snapshot == before && browser.ProjectId == "a", "reopening shows the existing project while rescanning");
        browser.Select("b");
        client.Pending.SetResult(json); await next;
        Check(browser.ProjectId == "b", "a selection made during the scan survives the fresh result");

        using var abandoned = new CancellationTokenSource();
        var old = browser.RefreshAsync(client, "scope", abandoned.Token); var lateReply = client.Pending;
        abandoned.Cancel();
        var fresh = browser.RefreshAsync(client, "scope", default);
        var removed = json.DeepClone(); removed["projects"]!.AsArray().RemoveAt(1);
        client.Pending.SetResult(removed); await fresh;
        Check(browser.ProjectId == null && browser.Snapshot!.Projects.Count == 1, "a removed selection falls back to All projects");
        browser.Select("a"); var current = browser.Snapshot;
        lateReply.SetResult(json);
        try { await old; throw new Exception("abandoned scan unexpectedly applied"); } catch (OperationCanceledException) { }
        Check(browser.Snapshot == current && browser.ProjectId == "a", "late replies from unmounted pages do not replace the new overview");
        var failed = browser.RefreshAsync(client, "scope", default); client.Pending.SetException(new IOException("scan unavailable"));
        try { await failed; throw new Exception("expected read failure"); } catch (IOException) { }
        Check(browser.Snapshot == current && browser.ProjectId == "a", "failed refresh retains the last successful selection and usage");
        Check(new NativeStorageBrowser().Snapshot == null, "another window or database starts without cached paths");
    }
    private sealed class ScanClient : INativeSettingsClient
    {
        public TaskCompletionSource<JsonNode?> Pending = new();
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(command == "get_storage_usage" && arguments.Count == 0 && projectId == "scope", "browsing uses only the read-only storage command");
            Pending = new(TaskCreationOptions.RunContinuationsAsynchronously);
            return Pending.Task;
        }
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
