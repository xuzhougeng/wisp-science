using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    public int FileReferenceRevision => generation;
    public bool CanReceiveFileReference => CanAttach && Snapshot?.ComposerReferences == true || CanPrepareProjectDraft;
    public int ReferenceClipboardPaths(IEnumerable<string> paths, int revision)
        => ReferenceLocalPaths(paths, revision);
    private int ReferenceLocalPaths(IEnumerable<string> paths, int revision)
    {
        if (revision != generation || !CanReceiveFileReference) return 0;
        var added = 0;
        foreach (var path in paths.Where(NativeFilePathRules.WindowsClipboardPath).Distinct(StringComparer.OrdinalIgnoreCase))
        {
            if (References.Any(item => item.Reference.Kind == "file-path" && string.Equals(item.Reference.Id, path, StringComparison.OrdinalIgnoreCase))) continue;
            var label = path.Replace('\\', '/').TrimEnd('/').Split('/').Last();
            if (AddReference(new(new("file-path", Id: path), label, path))) added++;
        }
        return added;
    }
    public async Task<int> ReferenceDroppedPathsAsync(Func<Task<IReadOnlyList<string>>> readPaths, CancellationToken token = default)
    {
        if (!CanReceiveFileReference || token.IsCancellationRequested) return 0;
        var revision = generation;
        try
        {
            var paths = await readPaths();
            return token.IsCancellationRequested ? 0 : ReferenceLocalPaths(paths, revision);
        }
        catch (OperationCanceledException) { return 0; }
        catch (Exception ex)
        {
            if (revision == generation && CanReceiveFileReference && !token.IsCancellationRequested)
                ReportAttachmentSelectionError(ex.Message);
            return 0;
        }
    }
    public bool CanReferenceFile(NativeFileDrag drag) => settings != null && CanReceiveFileReference && drag.Valid
        && drag.ProjectId == projectId && drag.SessionId == sessionId;
    public async Task<bool> ReferenceFileAsync(NativeFileDrag drag, CancellationToken token = default)
    {
        if (!CanReferenceFile(drag) || token.IsCancellationRequested) return false;
        var request = generation;
        try
        {
            var files = new NativeFilesClient(settings!);
            var locations = await files.LocationsAsync(drag.ProjectId, drag.SessionId, token);
            if (request != generation || !CanReferenceFile(drag) || token.IsCancellationRequested) return false;
            var result = await files.PathsAsync(drag.ProjectId, drag.SessionId, [drag.Path], locations.LocalRoot, token);
            if (request != generation || !CanReferenceFile(drag) || token.IsCancellationRequested) return false;
            var path = result.Paths.Single().RelativePath;
            return AddReference(new(new("file-path", Id: path), path.Split('/').Last(), path));
        }
        catch (OperationCanceledException) { return false; }
        catch (Exception ex) { if (request == generation) { OperationError = ex.Message; Notify(); } return false; }
    }
}
