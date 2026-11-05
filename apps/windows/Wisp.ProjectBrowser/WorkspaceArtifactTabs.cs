using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeArtifactTab(string Key, NativePanelArtifact Item, WorkspaceArtifactModel Reader);

/// <summary>Open center files for one workspace; closing a tab invalidates its pending reads.</summary>
public sealed class WorkspaceArtifactTabs : IDisposable
{
    private readonly List<NativeArtifactTab> items = [];
    private bool disposed;
    public IReadOnlyList<NativeArtifactTab> Items => items;
    public string? SelectedKey { get; private set; }
    public NativeArtifactTab? Selected => items.FirstOrDefault(tab => tab.Key == SelectedKey);
    public event Action? Changed;
    public async Task OpenAsync(WorkspaceArtifactModel reader, string id)
    {
        if (disposed) { reader.Close(); return; }
        var item = reader.Find(id) ?? throw new InvalidOperationException("Artifact is no longer available");
        var key = reader.Scope + ":" + item.Path;
        if (items.Any(tab => tab.Key == key)) { reader.Close(); Select(key); return; }
        var tab = new NativeArtifactTab(key, item, reader); items.Add(tab); SelectedKey = key;
        reader.Changed += Notify; Notify();
        await reader.OpenAsync(id);
    }
    public void Select(string? key) { if (disposed || key != null && items.All(tab => tab.Key != key)) return; SelectedKey = key; Notify(); }
    public void Close(string key)
    {
        if (items.FirstOrDefault(tab => tab.Key == key) is not { } tab) return;
        items.Remove(tab); tab.Reader.Changed -= Notify; tab.Reader.Close();
        if (SelectedKey == key) SelectedKey = null; Notify();
    }
    private void Notify() { if (!disposed) Changed?.Invoke(); }
    public void Dispose()
    {
        if (disposed) return; disposed = true;
        foreach (var tab in items) { tab.Reader.Changed -= Notify; tab.Reader.Close(); }
        items.Clear(); SelectedKey = null;
    }
}
