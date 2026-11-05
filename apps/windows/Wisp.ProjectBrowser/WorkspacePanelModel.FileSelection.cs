using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspacePanelModel
{
    private readonly HashSet<string> selectedFiles = [];
    private int fileCopyRequest;
    public bool SelectingFiles { get; private set; }
    public bool FileCopyBusy { get; private set; }
    public bool CanSelectFiles => !closed && !RemoteFiles && !Loading && !FileSearchLoading && fileClient != null && Tabs.Selected == "files";
    public IReadOnlySet<string> SelectedFiles => selectedFiles;
    private string[] VisibleFilePaths => SearchingFiles ? FileSearchHits.Select(h => h.Path).ToArray() : Files.Select(f => Child(Path, f.Name)).ToArray();
    private void InvalidateFileCopy() { fileCopyRequest++; FileCopyBusy = false; }
    public void ClearFileSelection(bool leaveMode = false)
    { selectedFiles.Clear(); InvalidateFileCopy(); if (leaveMode) SelectingFiles = false; }
    public void ToggleFileSelectionMode()
    {
        if (!CanSelectFiles) return;
        SelectingFiles = !SelectingFiles; ClearFileSelection();
    }
    public void ToggleFileSelection(string path)
    {
        if (!CanSelectFiles || !SelectingFiles || !VisibleFilePaths.Contains(path)) return;
        if (!selectedFiles.Add(path)) selectedFiles.Remove(path);
        InvalidateFileCopy();
    }
    public void SelectAllFiles()
    {
        if (!CanSelectFiles || !SelectingFiles) return;
        var visible = VisibleFilePaths;
        var all = selectedFiles.SetEquals(visible);
        ClearFileSelection(); if (!all) selectedFiles.UnionWith(visible);
    }
    public async Task<string?> CopyFilePathsAsync(bool absolute, string? clicked = null, CancellationToken token = default)
    {
        if (!CanSelectFiles || FileCopyBusy || FileLocations == null || token.IsCancellationRequested) return null;
        var visible = VisibleFilePaths;
        if (clicked != null && !visible.Contains(clicked)) return null;
        var paths = clicked != null && !selectedFiles.Contains(clicked) ? [clicked]
            : selectedFiles.Intersect(visible).Order(StringComparer.Ordinal).ToArray();
        if (paths.Length == 0) return null;
        var request = ++fileCopyRequest; var scope = generation; FileCopyBusy = true; Error = null;
        try
        {
            var result = await fileClient!.PathsAsync(projectId, sessionId, paths, FileLocations.LocalRoot, token);
            if (closed || token.IsCancellationRequested || request != fileCopyRequest || scope != generation || RemoteFiles) return null;
            return string.Join("\n", result.Paths.Select(p => absolute ? p.AbsolutePath : p.RelativePath));
        }
        catch (OperationCanceledException) { return null; }
        catch (Exception ex) { if (!closed && request == fileCopyRequest) Error = ex.Message; return null; }
        finally { if (request == fileCopyRequest) FileCopyBusy = false; }
    }
    public NativeFileDrag? FileDrag(string path)
    {
        if (!CanSelectFiles || !NativeFilePathRules.Relative(path)) return null;
        var file = SearchingFiles ? FileSearchHits.Any(h => h.Path == path && !h.IsDir)
            : Files.Any(f => Child(Path, f.Name) == path && !f.IsDir);
        return file ? new(projectId, sessionId, path) : null;
    }
}
