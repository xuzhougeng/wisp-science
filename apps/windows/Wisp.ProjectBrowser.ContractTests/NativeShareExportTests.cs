using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeShareExportTests
{
    public static async Task RunAsync()
    {
        Check(NativeShareDraft.Width("-1") == 840 && NativeShareDraft.Width("4294967295") == 2400
            && NativeShareDraft.Width("0") == 320 && NativeShareDraft.Width("4294967296") == 840, "width matches WebView u32 parsing");
        var client = new Client();
        using var model = new WorkspaceShareModel(client, "project", "session");
        await model.LoadAsync();
        Check(model.CanExport && model.Format == NativeShareFormat.Png && model.Selected.Length == 2, "PNG default and reasoning excluded");
        model.SelectAll(false); Check(!model.CanExport, "clear selection disables export");
        Check(!await model.ExportAsync(false, (_, _) => throw new Exception("must not save")), "empty export rejected");
        model.SelectAll(true); model.Keywords = "Sensitive"; model.Width = "1200";
        var pending = client.Pending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var saves = 0;
        var export = model.ExportAsync(false, (document, _) =>
        {
            saves++;
            Check(document.Format == NativeShareFormat.Png && document.Width == 1200, "format and width frozen");
            Check(document.Rows.All(r => !r.Text.Contains("Sensitive", StringComparison.OrdinalIgnoreCase)), "redaction applied to frozen rows");
            return Task.FromResult<string?>(null);
        });
        Check(model.Exporting && !model.CanExport, "single active export");
        Check(!await model.ExportAsync(false, (_, _) => throw new Exception("duplicate")), "duplicate export ignored");
        model.SelectAll(false); model.Format = NativeShareFormat.Html; model.Width = "600"; model.Keywords = "";
        pending.SetResult("html");
        Check(!await export && saves == 1 && model.Error == null && model.Rows.All(r => r.Selected), "save cancellation retains dialog and selection");
        client.Pending = null;
        Check(!await model.ExportAsync(false, (_, _) => throw new IOException("disk full")) && model.Error == "disk full", "save failure visible without retry");
        Check(await model.ExportAsync(false, (_, _) => Task.FromResult<string?>("saved.html")) && model.SavedPath == "saved.html", "explicit retry can succeed");

        var late = new Client { Pending = new(TaskCreationOptions.RunContinuationsAsynchronously) };
        var closed = new WorkspaceShareModel(late, "old-project", "old-session"); await closed.LoadAsync();
        var count = 0;
        var closing = closed.ExportAsync(false, (_, _) => { count++; return Task.FromResult<string?>("wrong"); });
        closed.Dispose(); late.Pending.SetResult("late");
        Check(!await closing && count == 0 && closed.SavedPath == null, "late HTML cannot open a picker after close");
        var loading = new Client { Read = new(TaskCreationOptions.RunContinuationsAsynchronously) };
        var abandoned = new WorkspaceShareModel(loading, "old-project", "old-session");
        var load = abandoned.LoadAsync(); abandoned.Dispose(); loading.Read.SetResult([new("user", "late")]); await load;
        Check(abandoned.Rows.Length == 0, "late initial read ignored after disposal");
    }
    private sealed class Client : INativeShareClient
    {
        public TaskCompletionSource<string>? Pending;
        public TaskCompletionSource<NativeShareRow[]>? Read;
        public Task<NativeShareRow[]> ReadAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) => Read?.Task
            ?? Task.FromResult<NativeShareRow[]>([new("user", "Sensitive question"), new("reasoning", "sensitive thought"), new("assistant", "SENSITIVE result")]);
        public Task<string> HtmlAsync(string projectId, string sessionId, NativeShareRow[] rows, bool dark = false, CancellationToken cancellationToken = default) => Pending?.Task ?? Task.FromResult("html");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException("Share export: " + message); }
}
