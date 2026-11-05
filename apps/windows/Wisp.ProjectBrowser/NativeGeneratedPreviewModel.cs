using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Read-only selection bound to the current artifact catalog.
/// Closing, navigation and supersession invalidate late reads without replaying them.</summary>
public sealed class NativeGeneratedPreviewModel(Func<NativeGeneratedOutput, CancellationToken, Task<NativePanelFileContent>>? read = null,
    Func<NativePanelArtifact, CancellationToken, Task<NativePanelFileContent>>? readRegistered = null)
{
    private IReadOnlyList<NativeArtifactEntry> source = [];
    private int generation;
    public NativeArtifactEntry? Selected { get; private set; }
    public NativePanelFileContent? Content { get; private set; }
    public string? Error { get; private set; }
    public bool Loading { get; private set; }
    public bool FileUnavailable => Selected?.DisplayPath != null
        && (Selected.Registered != null ? readRegistered == null : read == null);
    public event Action? Changed;
    public string? DocumentQuotePath(NativeArtifactEntry entry, NativePanelFileContent content, NativeDocumentSelection selection)
    {
        if (Loading || !ReferenceEquals(Selected, entry) || !ReferenceEquals(Content, content)
            || !NativeArtifactSelection.Allows(content, entry.Name, entry.Kind, selection)) return null;
        // A registered snapshot is not interchangeable with its displayed workspace alias.
        var path = entry.Registered is { } registered ? "artifact:" + registered.Id : entry.Output?.Path;
        return string.IsNullOrWhiteSpace(path) || path.Length > 32768 || path.Any(char.IsControl) ? null : path;
    }
    public void Bind(NativeGeneratedArtifacts outputs)
        => Bind(outputs.Items);
    public void Bind(IReadOnlyList<NativeGeneratedOutput> outputs)
        => BindCatalog(outputs.Select(item => new NativeArtifactEntry(item.Id, item.Name, item.Kind, "", "", item.Path, item, null)).ToArray());
    public void BindCatalog(IReadOnlyList<NativeArtifactEntry> outputs)
    {
        source = outputs;
        if (Selected is not { } old) return;
        var current = outputs.FirstOrDefault(item => item.Id == old.Id);
        if (current == null || current.Output?.Superseded == true) { Close(); return; }
        if (current.Output != old.Output || current.Registered != old.Registered) { Close(); return; }
        if (Selected != current) { Selected = current; Changed?.Invoke(); }
    }
    public async Task<bool> OpenAsync(string id, CancellationToken token = default)
    {
        if (token.IsCancellationRequested) return false;
        if (source.FirstOrDefault(item => item.Id == id) is not { } entry || entry.Output?.Superseded == true) return false;
        var request = ++generation; Selected = entry; Content = null; Error = null;
        Loading = entry.DisplayPath != null && !FileUnavailable;
        Changed?.Invoke();
        if (!Loading) return true;
        try
        {
            var result = entry.Registered is { } registered
                ? await readRegistered!(registered, token) : await read!(entry.Output!, token);
            if (request != generation || token.IsCancellationRequested) return false;
            Content = result;
        }
        catch (Exception ex)
        { if (request == generation && !token.IsCancellationRequested) Error = ex.Message; }
        finally
        { if (request == generation) { if (token.IsCancellationRequested) Close(); else { Loading = false; Changed?.Invoke(); } } }
        return request == generation && !token.IsCancellationRequested;
    }
    public Task<bool> RetryAsync(CancellationToken token = default) => Selected is { DisplayPath: not null } selected && !Loading
        ? OpenAsync(selected.Id, token) : Task.FromResult(false);
    public void Close() { generation++; Selected = null; Content = null; Error = null; Loading = false; Changed?.Invoke(); }
}
