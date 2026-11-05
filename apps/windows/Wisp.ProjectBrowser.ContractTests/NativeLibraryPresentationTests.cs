using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeLibraryPresentationTests
{
    public static async Task Run()
    {
        Check(NativeLibraryLayout.ForViewport(700).Compact && !NativeLibraryLayout.ForViewport(701).Compact, "WebView compact breakpoint");
        Check(NativeLibraryLayout.ForViewport(400).Inset == 18 && NativeLibraryLayout.ForViewport(1700).Inset == 48, "bounded page insets");
        var desktop = NativeLibraryLayout.ForViewport(1707);
        Check(desktop.ContentWidth == 1040 && desktop.CenterOffset + desktop.Inset == 333.5, "bounded desktop column centered on the viewport");
        var compact = NativeLibraryLayout.ForViewport(643);
        Check(compact.CenterOffset == 0 && Math.Abs(compact.ContentWidth + 2 * compact.Inset - 643) < .01, "compact column fits both insets");
        Check(NativeLibraryLayout.Excerpt("a\r\nb\nc\nd\ne") == "a\nb\nc\nd", "four original preview lines");
        var fake = new Fake(); using var model = new WorkspaceLibraryModel(fake);
        var old = new TaskCompletionSource<IReadOnlyList<LibraryItemSummary>>(); fake.Search = (_, _) => old.Task;
        var first = model.SearchAsync(); model.Query = "new";
        fake.Search = (_, kind) => Task.FromResult<IReadOnlyList<LibraryItemSummary>>([fake.Item.Summary]);
        await model.SearchAsync(); old.SetResult([]); await first;
        Check(model.Items.Count == 1 && !model.Busy, "late search cannot overwrite current results");
        await model.OpenAsync("item"); Check(model.Version?.VersionNumber == 2 && model.Selected?.Code == "original\n", "latest version selected; immutable original preserved");
        model.SelectVersion("item"); model.BeginEdit(); Check(model.Draft == "original\n", "edit seeds exact selected version");
        model.Draft = "new\nfull\ncode\n"; fake.FailSave = true; await model.SaveAsync();
        Check(model.Editing && model.Draft.EndsWith("code\n") && model.DetailError != null && fake.Writes == 1, "failed save retains multiline draft without replay");
        fake.FailSave = false; await model.SaveAsync(); Check(model.Version?.VersionNumber == 3 && !model.Editing && fake.Writes == 2, "explicit save appends and selects new version");
        var prompt = WorkspaceLibraryModel.ComposerText(fake.Item, model.Version!, "{title} v{version} {id}");
        Check(prompt.Contains("v3 item") && prompt.Contains("```python\nnew\nfull\ncode\n```"), "insert carries full version, identity and language");
        fake.FailDelete = true; await model.DeleteAsync("item"); Check(model.Selected != null && model.Items.Count == 1, "uncertain removal retains detail and row");
        fake.FailDelete = false; await model.DeleteAsync("item"); Check(model.Selected == null && model.Items.Count == 0, "confirmed removal closes only detail");
        var heldDetail = new TaskCompletionSource<LibraryItemDetail>(); fake.Detail = () => heldDetail.Task;
        var opening = model.OpenAsync("item"); model.CloseDetail(); heldDetail.SetResult(fake.Item); await opening;
        Check(model.Selected == null && !model.LoadingDetail, "Escape during read prevents late dialog reopening");
        fake.Detail = () => Task.FromResult(fake.Item with { Kind = "text" }); await model.OpenAsync("item"); model.BeginEdit();
        Check(!model.CanEdit && !model.Editing, "highlights remain verbatim");
        var heldSearch = new TaskCompletionSource<IReadOnlyList<LibraryItemSummary>>(); fake.Search = (_, _) => heldSearch.Task;
        var pending = model.SearchAsync(); model.Dispose(); heldSearch.SetResult([fake.Item.Summary]); await pending;
        Check(model.Items.Count == 0, "closed library ignores read replies");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeLibraryClient
    {
        public LibraryItemDetail Item = new("item", "code", "Analysis", "python", "original\n", null, "p", "P", "s", "S", null, 1, null);
        public Func<string, string?, Task<IReadOnlyList<LibraryItemSummary>>> Search = (_, _) => Task.FromResult<IReadOnlyList<LibraryItemSummary>>([]);
        public Func<Task<LibraryItemDetail>>? Detail;
        public bool FailSave, FailDelete; public int Writes;
        public Task<IReadOnlyList<LibraryItemSummary>> SearchAsync(string query, string? kind, CancellationToken cancellationToken = default) => Search(query, kind);
        public Task<LibraryItemDetail> GetAsync(string id, CancellationToken cancellationToken = default) => Detail?.Invoke() ?? Task.FromResult(Item);
        public Task<IReadOnlyList<LibraryItemVersion>> VersionsAsync(string id, CancellationToken cancellationToken = default) => Task.FromResult<IReadOnlyList<LibraryItemVersion>>([
            new("item", "item", 1, null, "python", "original\n", "original", 1), new("v2", "item", 2, "item", "python", "revised\n", "edit", 2)]);
        public Task<LibraryItemVersion> SaveCodeAsync(string id, string code, CancellationToken cancellationToken = default)
        { Writes++; if (FailSave) throw new IOException("Lost response"); return Task.FromResult(new LibraryItemVersion("v3", id, 3, "v2", "python", code, "edit", 3)); }
        public Task<bool> DeleteAsync(string id, CancellationToken cancellationToken = default)
        { if (FailDelete) throw new IOException("Lost delete"); return Task.FromResult(true); }
    }
}
