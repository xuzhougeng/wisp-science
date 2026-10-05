using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Reviewed file/destination identity and one-shot imports. Closing or
/// changing selection rejects late previews; uncertain writes are never replayed.</summary>
public sealed class WorkspaceArchiveImportModel(INativeSessionImportClient client, string initialProject) : IDisposable
{
    public event Action? Changed;
    public string Project { get; private set; } = initialProject;
    public string Path { get; private set; } = "";
    public NativeArchivePreview? Preview { get; private set; }
    public NativeArchiveImportResult? Result { get; private set; }
    public string? Error { get; private set; }
    public bool Reading { get; private set; }
    public bool Importing { get; private set; }
    public bool Uncertain { get; private set; }
    public bool Closed { get; private set; }
    public bool CanImport => !Closed && !Reading && !Importing && !Uncertain && Result == null && Preview != null;
    private int generation;
    private CancellationTokenSource? pending;

    public void Select(string project, string path)
    {
        if (Closed || Importing || (Project == project && Path == path)) return;
        generation++; pending?.Cancel(); Project = project; Path = path;
        Preview = null; Result = null; Error = null; Reading = false; Changed?.Invoke();
    }
    public async Task PreviewAsync()
    {
        if (Closed || Importing) return;
        var current = ++generation; pending?.Cancel();
        using var request = new CancellationTokenSource(); pending = request;
        var project = Project; var path = Path;
        Preview = null; Result = null; Error = null; Reading = true; Changed?.Invoke();
        try
        {
            if (string.IsNullOrWhiteSpace(project) || !System.IO.Path.IsPathFullyQualified(path)) throw new InvalidOperationException("请选择目标项目和会话 ZIP 归档。");
            var preview = await client.PreviewAsync(project, path, request.Token);
            if (Closed || current != generation) return;
            if (preview.ProjectId != project || preview.ArchivePath != path) throw new InvalidDataException("预览与当前文件或目标项目不符。");
            Preview = preview;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!Closed && current == generation) Error = ex.Message; }
        finally
        {
            if (ReferenceEquals(pending, request)) pending = null;
            if (!Closed && current == generation) { Reading = false; Changed?.Invoke(); }
        }
    }
    public async Task ImportAsync()
    {
        if (!CanImport || Preview is not { } reviewed) return;
        Importing = true; Error = null; Changed?.Invoke();
        try
        {
            var result = await client.ImportAsync(reviewed);
            if (Closed) return;
            if (result.ProjectId != reviewed.ProjectId || result.SourceSessionId != reviewed.SourceSessionId || string.IsNullOrWhiteSpace(result.FrameId))
                throw new InvalidDataException("导入结果与已确认的目标不符。");
            Result = result;
        }
        catch (Exception ex)
        {
            if (!Closed) { Uncertain = true; Error = "导入结果未能确认，请先在目标项目核对；本窗口不会再次提交。\n" + ex.Message; }
        }
        finally { Importing = false; if (!Closed) Changed?.Invoke(); }
    }
    public void Dispose() { Closed = true; generation++; pending?.Cancel(); pending = null; Reading = false; }
}
