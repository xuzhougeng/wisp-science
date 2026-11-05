using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class NativeMemoDraft
{
    private readonly string originalTitle, originalBody;
    private string body = "";
    public NativeMemoDraft(string? id = null, string title = "", string body = "")
    { Id = id; Title = originalTitle = title; Body = body; originalBody = Body; }
    public string? Id { get; }
    public string Title { get; set; }
    // WinUI TextBox emits CR-only line endings; the shared editor/store uses LF.
    public string Body { get => body; set => body = value.ReplaceLineEndings("\n"); }
    public bool Dirty => Title != originalTitle || Body != originalBody;
    public bool CanSave => !string.IsNullOrWhiteSpace(Body);
    public int EditorRows => Math.Clamp(Body.Replace("\r\n", "\n").Split('\n').Length + (Body.EndsWith('\n') ? 0 : 1), 6, 24);
}

/// One project's private notes and unsaved editor. Retained across panel/session
/// changes in that project; closed when its owning project/window is replaced.
public sealed class WorkspaceMemosModel(INativeMemosClient client, string project) : IDisposable
{
    public string ProjectId { get; } = project;
    public NativeMemo[] Memos { get; private set; } = [];
    public NativeMemoDraft? Draft { get; private set; }
    public string Query { get; set; } = "";
    public string? Expanded { get; set; }
    public string? Error { get; private set; }
    public string? SaveError { get; private set; }
    public bool Loaded { get; private set; }
    public bool Loading { get; private set; }
    public bool Busy { get; private set; }
    public bool ConfirmDiscard { get; private set; }
    // An uncertain write is never resubmitted automatically (especially Add).
    public bool Uncertain { get; private set; }
    public event Action? Changed;
    private readonly CancellationTokenSource lifetime = new();
    private int revision;
    private bool disposed;
    public NativeMemo[] Filtered => Memos.Where(m => m.Title.Contains(Query.Trim(), StringComparison.OrdinalIgnoreCase)
        || m.Body.Contains(Query.Trim(), StringComparison.OrdinalIgnoreCase)).ToArray();
    public static string? Heading(NativeMemo memo) => !string.IsNullOrWhiteSpace(memo.Title) ? memo.Title.Trim()
        : memo.Body.ReplaceLineEndings("\n").Split('\n').Select(line => line.Trim()).FirstOrDefault(line => line.Length > 0);
    public static string ComposeInsert(string current, string body) => string.IsNullOrWhiteSpace(current) ? body : current.TrimEnd() + "\n\n" + body;
    private void Notify() { if (!disposed) Changed?.Invoke(); }
    private void Sort() => Memos = Memos.OrderByDescending(m => m.Pinned).ThenByDescending(m => m.UpdatedAt).ThenByDescending(m => m.Id, StringComparer.Ordinal).ToArray();
    public async Task LoadAsync()
    {
        if (disposed || Loading || Busy) return;
        var version = ++revision; Loading = true; Error = null; Notify();
        try
        {
            var rows = await client.ListAsync(ProjectId, lifetime.Token);
            if (disposed || version != revision) return;
            Memos = rows; Sort(); Loaded = true;
            if (Draft == null) Uncertain = false;
            // Keep an uncertain Add draft blocked until explicitly discarded, so
            // a read that contains an acknowledged duplicate cannot trigger retry.
        }
        catch (Exception ex) { if (!disposed && version == revision) Error = ex.Message; }
        finally { if (!disposed && version == revision) { Loading = false; Notify(); } }
    }
    public void New()
    { if (disposed || Draft != null || Busy) return; Draft = new(); SaveError = null; ConfirmDiscard = false; Uncertain = false; Notify(); }
    public void Edit(NativeMemo memo)
    {
        if (disposed || Draft != null || Busy || memo.ProjectId != ProjectId || !Memos.Any(m => m.Id == memo.Id)) return;
        Draft = new(memo.Id, memo.Title, memo.Body); SaveError = null; ConfirmDiscard = false; Uncertain = false; Notify();
    }
    public bool Escape()
    {
        if (ConfirmDiscard) { ConfirmDiscard = false; Notify(); return true; }
        if (Draft == null) return false;
        Cancel(); return true;
    }
    public void Cancel()
    {
        if (Busy) return;
        if (Draft?.Dirty == true) { ConfirmDiscard = true; Notify(); }
        else Discard();
    }
    public void KeepEditing() { ConfirmDiscard = false; Notify(); }
    public void Discard()
    { if (Busy) return; Draft = null; SaveError = null; ConfirmDiscard = false; Uncertain = false; Notify(); }
    public async Task SaveAsync()
    {
        if (Draft is not { CanSave: true } draft || Busy || Uncertain || disposed) return;
        await WriteAsync(async () => draft.Id is { } id
            ? await client.UpdateAsync(ProjectId, new(id, draft.Title, draft.Body), lifetime.Token)
            : await client.AddAsync(ProjectId, new(draft.Title, draft.Body), lifetime.Token), draft);
    }
    public Task PinAsync(NativeMemo memo) => memo.ProjectId == ProjectId && Memos.Any(m => m.Id == memo.Id)
        ? WriteAsync(() => client.UpdateAsync(ProjectId, new(memo.Id, Pinned: !memo.Pinned), lifetime.Token)) : Task.CompletedTask;
    private async Task WriteAsync(Func<Task<NativeMemo>> action, NativeMemoDraft? draft = null)
    {
        if (disposed || Busy || Uncertain) return;
        ++revision; Loading = false; Busy = true; Error = SaveError = null; Notify();
        try
        {
            var saved = await action(); if (disposed) return;
            Memos = [.. Memos.Where(m => m.Id != saved.Id), saved]; Sort();
            if (draft != null && ReferenceEquals(Draft, draft)) { Draft = null; ConfirmDiscard = false; }
        }
        catch (Exception ex) { if (!disposed) { Uncertain = true; if (draft != null) SaveError = ex.Message; else Error = ex.Message; } }
        finally { Busy = false; Notify(); }
    }
    public async Task DeleteAsync(NativeMemo memo)
    {
        if (disposed || Busy || Uncertain || memo.ProjectId != ProjectId) return;
        ++revision; Loading = false; Busy = true; Error = null; Notify();
        try { await client.DeleteAsync(ProjectId, memo.Id, lifetime.Token); if (!disposed) Memos = Memos.Where(m => m.Id != memo.Id).ToArray(); }
        catch (Exception ex) { if (!disposed) { Uncertain = true; Error = ex.Message; } }
        finally { Busy = false; Notify(); }
    }
    public void Dispose() { disposed = true; revision++; lifetime.Cancel(); lifetime.Dispose(); Draft = null; Memos = []; }
}
