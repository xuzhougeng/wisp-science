using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Independent read generations keep live search and detail navigation responsive.
/// Writes are dispatched once; failures retain the selected item and editor draft.
public sealed class WorkspaceLibraryModel(INativeLibraryClient client) : IDisposable
{
    public event Action? Changed;
    private string query = "";
    private string? kind;
    private int searchEpoch, detailEpoch;
    public bool Closed { get; private set; }
    public bool Busy { get; private set; }
    public bool LoadingDetail { get; private set; }
    public bool MutationBusy { get; private set; }
    public string? Error { get; private set; }
    public string? DetailError { get; private set; }
    public string Query { get => query; set { if (query == value) return; query = value; InvalidateSearch(); } }
    public string? Kind { get => kind; set { value = string.IsNullOrEmpty(value) || value == "all" ? null : value; if (kind == value) return; kind = value; InvalidateSearch(); } }
    public IReadOnlyList<LibraryItemSummary> Items { get; private set; } = [];
    public LibraryItemDetail? Selected { get; private set; }
    public IReadOnlyList<LibraryItemVersion> Versions { get; private set; } = [];
    public LibraryItemVersion? Version { get; private set; }
    public bool Editing { get; private set; }
    public string Draft { get; set; } = "";
    public bool CanEdit => Selected is { Kind: not "text" } && Version != null && !LoadingDetail && !MutationBusy;
    public bool CanSave => !Closed && CanEdit && Editing && !string.IsNullOrWhiteSpace(Draft);
    private void Notify() { if (!Closed) Changed?.Invoke(); }
    private void InvalidateSearch() { searchEpoch++; Items = []; Error = null; Busy = false; Notify(); }
    public async Task<bool> SearchAsync()
    {
        if (Closed) return false;
        var epoch = ++searchEpoch; Busy = true; Error = null; Notify();
        try
        {
            var rows = await client.SearchAsync(Query, Kind);
            if (Closed || searchEpoch != epoch) return false;
            Items = rows; return true;
        }
        catch (Exception ex) { if (!Closed && searchEpoch == epoch) Error = ex.Message; return false; }
        finally { if (!Closed && searchEpoch == epoch) { Busy = false; Notify(); } }
    }
    public async Task OpenAsync(string id)
    {
        if (Closed || MutationBusy) return;
        var epoch = ++detailEpoch; Selected = null; Versions = []; Version = null;
        Editing = false; LoadingDetail = true; DetailError = null; Notify();
        try
        {
            var detail = await client.GetAsync(id);
            if (Closed || detailEpoch != epoch) return;
            Selected = detail;
            var seed = new LibraryItemVersion(detail.Id, detail.Id, 1, null, detail.Language, detail.Code, "original", detail.CreatedAt);
            Versions = [seed]; Version = seed;
            if (detail.Kind != "text")
            {
                var versions = await client.VersionsAsync(id);
                if (Closed || detailEpoch != epoch) return;
                if (versions.Count > 0) { Versions = versions.OrderBy(v => v.VersionNumber).ToArray(); Version = Versions[^1]; }
            }
        }
        catch (Exception ex) { if (!Closed && detailEpoch == epoch) DetailError = ex.Message; }
        finally { if (!Closed && detailEpoch == epoch) { LoadingDetail = false; Notify(); } }
    }
    public void CloseDetail()
    {
        detailEpoch++; LoadingDetail = false; Selected = null; Version = null; Versions = [];
        Editing = false; Draft = ""; DetailError = null; Notify();
    }
    public void SelectVersion(string id)
    {
        if (Closed || MutationBusy || LoadingDetail) return;
        if (Versions.FirstOrDefault(v => v.Id == id) is not { } version) return;
        Version = version; Editing = false; Draft = ""; DetailError = null; Notify();
    }
    public void BeginEdit() { if (Closed || !CanEdit) return; Draft = Version!.Code; Editing = true; DetailError = null; Notify(); }
    public void CancelEdit() { if (Closed || MutationBusy) return; Editing = false; Draft = ""; DetailError = null; Notify(); }
    public async Task<bool> SaveAsync()
    {
        if (!CanSave || Selected == null) return false;
        var epoch = detailEpoch; var id = Selected.Id; var draft = Draft;
        MutationBusy = true; DetailError = null; Notify();
        try
        {
            var saved = await client.SaveCodeAsync(id, draft);
            if (Closed || detailEpoch != epoch) return false;
            Versions = Versions.Where(v => v.Id != saved.Id).Append(saved).OrderBy(v => v.VersionNumber).ToArray();
            Version = saved; Editing = false; Draft = ""; return true;
        }
        catch (Exception ex) { if (!Closed && detailEpoch == epoch) DetailError = ex.Message; return false; }
        finally { MutationBusy = false; Notify(); }
    }
    public async Task<bool> DeleteAsync(string id)
    {
        if (Closed || MutationBusy) return false;
        var epoch = detailEpoch; MutationBusy = true; DetailError = null; Error = null; Notify();
        try
        {
            if (!await client.DeleteAsync(id)) throw new InvalidOperationException("Library item no longer exists.");
            if (Closed) return false;
            searchEpoch++; Busy = false; Items = Items.Where(i => i.Id != id).ToArray();
            if (detailEpoch == epoch && Selected?.Id == id) CloseDetail();
            return true;
        }
        catch (Exception ex)
        {
            if (!Closed && detailEpoch == epoch) { if (Selected != null) DetailError = ex.Message; else Error = ex.Message; }
            return false;
        }
        finally { MutationBusy = false; Notify(); }
    }
    public static string ComposerText(LibraryItemDetail item, LibraryItemVersion version, string localizedPrompt)
        => localizedPrompt.Replace("{title}", item.Title, StringComparison.Ordinal)
            .Replace("{id}", item.Id, StringComparison.Ordinal).Replace("{version}", version.VersionNumber.ToString(System.Globalization.CultureInfo.InvariantCulture), StringComparison.Ordinal)
            + $"\n```{version.Language}\n{version.Code.TrimEnd()}\n```";
    // Kept for existing caller contracts; the detail UI always inserts a full version.
    public static string ComposerText(LibraryItemSummary item) => string.IsNullOrWhiteSpace(item.CodePreview)
        ? $"请看收藏「{item.Title}」。" : $"请用收藏「{item.Title}」重新运行：\n```{item.Language}\n{item.CodePreview.Trim()}\n```";
    public void Dispose() { Closed = true; searchEpoch++; detailEpoch++; Busy = LoadingDetail = false; }
}

public readonly record struct NativeLibraryLayout(bool Compact, double Inset, double ContentWidth, double CenterOffset)
{
    public static NativeLibraryLayout ForViewport(double width)
    {
        var inset = Math.Clamp(width * .04, 18, 48);
        var available = Math.Max(0, width - inset * 2);
        var content = Math.Min(1040, available);
        return new(width <= 700, inset, content, (available - content) / 2);
    }
    public static string Excerpt(string text) => string.Join("\n", text.ReplaceLineEndings("\n").Split('\n').Take(4));
}
