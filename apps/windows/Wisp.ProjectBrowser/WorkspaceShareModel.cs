using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public enum NativeShareFormat { Png, Html }
public sealed record NativeShareDocument(NativeShareFormat Format, int Width, string Html, NativeShareRow[] Rows);
public sealed record NativeShareFile(string Name, string Extension, byte[] Contents);

/// <summary>One dialog owns one fixed conversation and one export at a time.</summary>
public sealed class WorkspaceShareModel(INativeShareClient client, string projectId, string sessionId) : IDisposable
{
    private readonly CancellationTokenSource lifetime = new();
    private bool disposed;
    public event Action? Changed;
    public NativeShareDraftRow[] Rows { get; private set; } = [];
    public string Keywords { get; set; } = "";
    public string Width { get; set; } = "";
    public NativeShareFormat Format { get; set; }
    public bool Loading { get; private set; }
    public bool Exporting { get; private set; }
    public string? Error { get; private set; }
    public string? SavedPath { get; private set; }
    public NativeShareRow[] Selected => NativeShareDraft.Selected(Rows, Keywords);
    public bool CanExport => !disposed && !Loading && !Exporting && Rows.Any(row => row.Selected);

    public async Task LoadAsync(CancellationToken cancellationToken = default)
    {
        if (disposed || Loading || Exporting) return;
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token, cancellationToken);
        Loading = true; Error = null; Changed?.Invoke();
        try
        {
            var rows = await client.ReadAsync(projectId, sessionId, linked.Token);
            if (!disposed && !linked.IsCancellationRequested) Rows = NativeShareDraft.From(rows);
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!disposed) Error = ex.Message; }
        finally { if (!disposed) { Loading = false; Changed?.Invoke(); } }
    }

    public void SelectAll(bool selected)
    {
        if (disposed || Exporting) return;
        Rows = Rows.Select(row => row with { Selected = selected }).ToArray(); Changed?.Invoke();
    }

    public void SetSelected(int id, bool selected)
    {
        if (disposed || Exporting) return;
        Rows = Rows.Select(row => row.Id == id ? row with { Selected = selected } : row).ToArray(); Changed?.Invoke();
    }

    public void Edit(int id, string text)
    {
        if (disposed || Exporting) return;
        Rows = Rows.Select(row => row.Id == id ? row with { Row = row.Row with { Text = text } } : row).ToArray(); Changed?.Invoke();
    }

    public Task<string> HtmlAsync(bool dark, CancellationToken cancellationToken = default) =>
        client.HtmlAsync(projectId, sessionId, Selected, dark, cancellationToken);

    public async Task<bool> ExportAsync(bool dark, Func<NativeShareDocument, CancellationToken, Task<string?>> save)
    {
        if (!CanExport) return false;
        // Freeze before awaiting: edits and navigation cannot change this export's scope or contents.
        var rows = Selected; var format = Format; var width = NativeShareDraft.Width(Width);
        var token = lifetime.Token;
        Exporting = true; Error = null; SavedPath = null; Changed?.Invoke();
        try
        {
            var html = await client.HtmlAsync(projectId, sessionId, rows, dark, token);
            if (disposed || token.IsCancellationRequested) return false;
            var path = await save(new(format, width, html, rows), token);
            if (disposed || token.IsCancellationRequested) return false;
            SavedPath = string.IsNullOrEmpty(path) ? null : path;
            return SavedPath != null;
        }
        catch (OperationCanceledException) { return false; }
        catch (Exception ex) { if (!disposed) Error = ex.Message; return false; }
        finally { if (!disposed) { Exporting = false; Changed?.Invoke(); } }
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; lifetime.Cancel(); lifetime.Dispose(); Changed = null;
    }
}
