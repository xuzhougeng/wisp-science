using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspacePanelModel
{
    private readonly INativeFilesClient? fileClient;
    private string localDirectory = ".";
    private int fileSearchRequest, fileTransferRequest;
    public string FileSearchQuery { get; private set; } = "";
    public NativeFileSearchHit[] FileSearchHits { get; private set; } = [];
    public bool FileSearchLoading { get; private set; }
    public string? FileSearchError { get; private set; }
    public bool SearchingFiles => !RemoteFiles && FileSearchQuery.Trim().Length > 0;
    public bool RecursiveFileSearch => fileClient != null;
    public async Task RevealArtifactInputAsync(string path, CancellationToken token = default)
    {
        if (closed || string.IsNullOrWhiteSpace(path)) return;
        FileContext = "local"; ClearFileSearch(); ClearFileSelection(leaveMode: true);
        Preview = null; PreviewEditable = false;
        await RefreshAsync("files", ParentPath(path.Replace('\\', '/')), token);
    }
    public void ClearFileSearch()
    { fileSearchRequest++; FileSearchQuery = ""; FileSearchHits = []; FileSearchLoading = false; FileSearchError = null; ClearFileSelection(); }
    public async Task SearchFilesAsync(string query, CancellationToken token = default)
    {
        if (closed || RemoteFiles || fileClient == null || Tabs.Selected != "files") return;
        var request = ++fileSearchRequest;
        ClearFileSelection();
        FileSearchQuery = query; FileSearchHits = []; FileSearchError = null;
        FileSearchLoading = query.Trim().Length > 0;
        if (!FileSearchLoading) return;
        try
        {
            if (System.Text.Encoding.UTF8.GetByteCount(query.Trim()) > 512) throw new ArgumentException("Search is limited to 512 UTF-8 bytes.");
            var hits = await fileClient.SearchAsync(projectId, sessionId, query.Trim(), token);
            if (!closed && request == fileSearchRequest) FileSearchHits = hits;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!closed && request == fileSearchRequest) FileSearchError = ex.Message; }
        finally { if (request == fileSearchRequest) FileSearchLoading = false; }
    }
    public NativeFileLocations? FileLocations { get; private set; }
    public string FileContext { get; private set; } = "local";
    public bool RemoteFiles => FileContext != "local";
    public bool FileTransferBusy { get; private set; }
    public bool FileTransferUncertain { get; private set; }
    public NativeFileTransfer? FileTransfer { get; private set; }
    public bool FileTransferIsDownload { get; private set; }
    public Dictionary<string, NativeRun> FileTransferRuns { get; } = [];
    public string? FileTransferReadError { get; private set; }
    public bool HasPendingFileTransfers => FileTransfer?.Items.Any(i => i.RunId != null && !WorkspaceConversationModel.RunTerminal(i.Status)) == true;
    public bool CanUploadFiles => !closed && !Loading && !FileTransferBusy && !FileActionBusy
        && !HasPendingFileTransfers && !SearchingFiles && !FileTransferUncertain && FileLocations is { ReadOnly: false } && Tabs.Selected == "files";
    public bool CanDownloadFiles => !closed && !Loading && !FileTransferBusy && !FileActionBusy && !HasPendingFileTransfers
        && !FileTransferUncertain && RemoteFiles && FileLocations != null && Tabs.Selected == "files";
    public bool CanMutateLocalFiles => !closed && !Loading && !FileActionBusy && !FileTransferBusy && !RemoteFiles
        && !SearchingFiles && (fileClient == null || FileLocations is { ReadOnly: false }) && Tabs.Selected == "files";
    public static string RemoteParent(string path)
    {
        var clean = path.TrimEnd('/');
        if (clean is "" or "~") return path;
        var index = clean.LastIndexOf('/');
        return index == 0 ? "/" : index < 0 ? "~" : clean[..index];
    }
    private async Task RefreshFilesAsync(int current, string requested, CancellationToken token)
    {
        var context = FileContext;
        var locations = await fileClient!.LocationsAsync(projectId, sessionId, token);
        if (current != generation) return;
        FileLocations = locations;
        if (locations.ReadOnly) { CanEditPreview = false; PreviewEditable = false; }
        if (!locations.Locations.Any(l => l.ContextId == context))
        {
            Files = [];
            throw new InvalidOperationException("File location is no longer available; choose another location.");
        }
        var result = await fileClient.DirectoryAsync(projectId, sessionId, context, requested, token);
        if (current != generation) return;
        Files = result.Entries; Path = result.Path;
        selectedFiles.IntersectWith(VisibleFilePaths); InvalidateFileCopy();
        if (!RemoteFiles) localDirectory = Path;
        // Only an explicit fresh listing acknowledges an uncertain transfer.
        if (!FileTransferBusy) { FileTransferUncertain = false; FileTransferReadError = null; }
    }
    public async Task SelectFileLocationAsync(string context, CancellationToken token = default)
    {
        if (closed || FileTransferBusy || FileActionBusy || context == FileContext
            || FileLocations?.Locations.Any(l => l.ContextId == context) != true) return;
        FileContext = context; Files = []; Preview = null; PreviewEditable = false;
        ClearFileSearch(); ClearFileSelection(leaveMode: true); FileTransferUncertain = false;
        Path = RemoteFiles ? "~" : localDirectory;
        await RefreshAsync("files", Path, token);
    }
    /// <summary>Capture before opening the OS picker, so a late picker result cannot target another folder.</summary>
    public FileDestination? CaptureFileDestination() => CanUploadFiles ? new(generation, FileContext, Path) : null;
    public sealed record FileDestination(int Revision, string Context, string Path);
    public async Task UploadFilesAsync(FileDestination destination, IReadOnlyList<string> sources, CancellationToken token = default)
    {
        if (!CanUploadFiles || sources.Count == 0 || token.IsCancellationRequested || destination.Revision != generation
            || destination.Context != FileContext || destination.Path != Path) return;
        await TransferFilesAsync(false, destination, t => fileClient!.UploadAsync(projectId, sessionId, destination.Context, destination.Path, sources, t), token);
    }
    public FileDestination? CaptureFileDownload() => CanDownloadFiles ? new(generation, FileContext, Path) : null;
    public async Task DownloadFileAsync(FileDestination source, string path, string? destination, CancellationToken token = default)
    {
        if (!CanDownloadFiles || string.IsNullOrWhiteSpace(destination) || token.IsCancellationRequested
            || source.Revision != generation || source.Context != FileContext || source.Path != Path
            || !Files.Any(f => !f.IsDir && Child(Path, f.Name) == path)) return;
        await TransferFilesAsync(true, source, t => fileClient!.DownloadAsync(projectId, sessionId, source.Context, path, destination, t), token);
    }
    private async Task TransferFilesAsync(bool download, FileDestination location, Func<CancellationToken, Task<NativeFileTransfer>> submit, CancellationToken token)
    {
        FileTransferBusy = true; FileTransfer = null; FileTransferRuns.Clear(); FileTransferReadError = null; Error = null;
        FileTransferIsDownload = download;
        var current = ++fileTransferRequest;
        try
        {
            var result = await submit(token);
            if (current != fileTransferRequest || closed) return;
            FileTransfer = result;
            if (!download && FileContext == location.Context && Path == location.Path && Tabs.Selected == "files")
                await RefreshAsync("files", cancellationToken: token);
        }
        catch (Exception ex)
        {
            if (current == fileTransferRequest && !closed)
            {
                FileTransferUncertain = true;
                Error = "文件传输结果未确认，请刷新核对；不会自动重试。 / Transfer result unconfirmed. Refresh to check before trying again.\n" + ex.Message;
            }
        }
        finally { FileTransferBusy = false; }
    }
    public async Task RefreshFileTransfersAsync(CancellationToken token = default)
    {
        if (closed || fileClient == null || FileTransferBusy || FileTransfer is not { } transfer || !HasPendingFileTransfers) return;
        var request = fileTransferRequest;
        try
        {
            var runs = new List<NativeRun>();
            foreach (var item in transfer.Items.Where(i => i.RunId != null && !WorkspaceConversationModel.RunTerminal(i.Status)))
            {
                var run = await fileClient.TransferRunAsync(projectId, sessionId, transfer.ContextId, item.RunId!, token);
                if (run.Id != item.RunId || run.ContextId != transfer.ContextId || run.FrameId != sessionId || run.Kind != "file_transfer")
                    throw new InvalidDataException("File transfer identity mismatch");
                runs.Add(run);
            }
            if (closed || request != fileTransferRequest) return;
            foreach (var run in runs) FileTransferRuns[run.Id] = run;
            FileTransfer = transfer with { Items = transfer.Items.Select(i => i.RunId is { } id && FileTransferRuns.TryGetValue(id, out var run)
                ? i with { Status = run.Status, Error = run.LastPollError ?? (run.Status == "failed" ? run.StderrTail : null) } : i).ToArray() };
            FileTransferReadError = null;
            if (!FileTransferIsDownload && !HasPendingFileTransfers && FileContext == transfer.ContextId && Path == transfer.Path && Tabs.Selected == "files")
                await RefreshAsync("files", cancellationToken: token);
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!closed && request == fileTransferRequest) FileTransferReadError = ex.Message; }
    }
}
