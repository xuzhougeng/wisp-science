using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>One source/destination per batch; read generations reject stale data
/// and an ambiguous write stops the batch without replaying the current item.</summary>
public sealed class WorkspaceExternalImportModel(INativeExternalImportClient client, string project) : IDisposable
{
    public const int PageSize = 25;
    public event Action? Changed;
    public string Project { get; private set; } = project;
    public string Provider { get; private set; } = "codex";
    public string Context { get; private set; } = "local";
    public ExternalImportSource[] Sources { get; private set; } = [new("local", "本地", "local")];
    public ExternalImportItem[] Items { get; private set; } = [];
    public Dictionary<string, ExternalImportResult> Results { get; } = [];
    public Dictionary<string, string> ItemErrors { get; } = [];
    public ExternalImportPreview? Preview { get; private set; }
    public string? SelectedPath { get; private set; }
    public string Query { get; private set; } = "";
    public int Page { get; private set; }
    public ExternalImportItem[] Filtered => Items.Where(i => (i.Title + " " + i.Cwd + " " + i.SessionId + " " + i.Path).Contains(Query.Trim(), StringComparison.OrdinalIgnoreCase)).ToArray();
    public int PageCount => Math.Max(1, (Filtered.Length + PageSize - 1) / PageSize);
    public ExternalImportItem[] PageItems => Filtered.Skip(Page * PageSize).Take(PageSize).ToArray();
    public bool Loading { get; private set; }
    public bool Previewing { get; private set; }
    public bool Importing { get; private set; }
    public bool Uncertain { get; private set; }
    public bool StopRequested { get; private set; }
    public bool Closed { get; private set; }
    public string? Error { get; private set; }
    public string? SourcesError { get; private set; }
    public int Done { get; private set; }
    public int Total { get; private set; }
    public int Imported { get; private set; }
    public int Updated { get; private set; }
    public int Skipped { get; private set; }
    public int Failed { get; private set; }
    public bool CanImport => !Closed && !Loading && !Previewing && !Importing && !Uncertain && Preview != null && !Results.ContainsKey(Preview.Path);
    public bool CanImportFiltered => !Closed && !Loading && !Previewing && !Importing && !Uncertain && Filtered.Any(i => i.State != "imported" && !Results.ContainsKey(i.Path));
    private int selection, listGeneration, previewGeneration;
    private readonly CancellationTokenSource lifetime = new();
    private CancellationTokenSource? listRead, previewRead;

    public async Task InitializeAsync()
    {
        if (Closed || Importing) return;
        var current = selection; var destination = Project;
        try
        {
            var response = await client.SourcesAsync(destination, lifetime.Token);
            if (Closed || current != selection) return;
            if (response.ProjectId != destination) throw new InvalidDataException("来源列表与当前目标项目不符。");
            Sources = response.Sources; SourcesError = null;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!Closed && current == selection) SourcesError = ex.Message; }
        if (!Closed && current == selection) { Notify(); await LoadAsync(false); }
    }
    public void Select(string destination, string provider, string context)
    {
        if (Closed || Importing || (Project == destination && Provider == provider && Context == context)) return;
        selection++; listGeneration++; listRead?.Cancel(); InvalidatePreview();
        Project = destination; Provider = provider; Context = context; Items = []; Results.Clear(); ItemErrors.Clear();
        Page = 0; Loading = false; Error = null; Done = Total = Imported = Updated = Skipped = Failed = 0; Notify();
    }
    public void SetQuery(string query)
    {
        if (Closed || Importing || Query == query) return;
        Query = query; Page = 0; InvalidatePreview(); Notify();
    }
    public void SetPage(int page)
    {
        if (Closed || Importing) return;
        Page = Math.Clamp(page, 0, PageCount - 1); InvalidatePreview(); Notify();
    }
    public void ClosePreview() { if (!Closed && !Importing) { InvalidatePreview(); Notify(); } }
    private void InvalidatePreview()
    {
        previewGeneration++; previewRead?.Cancel(); Preview = null; SelectedPath = null; Previewing = false;
    }
    public async Task LoadAsync(bool refresh)
    {
        if (Closed || Importing) return;
        var current = ++listGeneration; var destination = Project; var provider = Provider; var context = Context;
        listRead?.Cancel(); using var read = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token); listRead = read;
        InvalidatePreview(); Loading = true; Items = []; Results.Clear(); ItemErrors.Clear(); Page = 0; Error = null; Notify();
        try
        {
            var response = await client.ListAsync(destination, provider, context, refresh, read.Token);
            if (Closed || current != listGeneration) return;
            if (response.ProjectId != destination || response.Provider != provider || response.ContextId != context) throw new InvalidDataException("会话列表与当前来源或目标不符。");
            Items = response.Items;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!Closed && current == listGeneration) Error = ex.Message; }
        finally { if (ReferenceEquals(listRead, read)) listRead = null; if (!Closed && current == listGeneration) { Loading = false; Notify(); } }
    }
    public async Task PreviewAsync(ExternalImportItem item)
    {
        if (Closed || Loading || Importing || !Items.Any(i => i.Path == item.Path && i.SessionId == item.SessionId)) return;
        InvalidatePreview(); var current = previewGeneration; SelectedPath = item.Path; Previewing = true; Error = null;
        using var read = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token); previewRead = read; Notify();
        try
        {
            var reviewed = await client.PreviewAsync(Project, Provider, Context, item, read.Token);
            if (Closed || current != previewGeneration) return;
            ValidatePreview(reviewed, item); Preview = reviewed;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!Closed && current == previewGeneration) Error = ex.Message; }
        finally { if (ReferenceEquals(previewRead, read)) previewRead = null; if (!Closed && current == previewGeneration) { Previewing = false; Notify(); } }
    }
    private void ValidatePreview(ExternalImportPreview reviewed, ExternalImportItem item)
    {
        if (reviewed.ProjectId != Project || reviewed.Provider != Provider || reviewed.ContextId != Context || reviewed.Path != item.Path || reviewed.SourceSessionId != item.SessionId)
            throw new InvalidDataException("预览的会话或来源已改变，请重新扫描。");
    }
    public Task ImportPreviewAsync() => CanImport && Preview is { } reviewed
        ? ImportBatchAsync(Items.Where(i => i.Path == reviewed.Path).ToArray(), reviewed) : Task.CompletedTask;
    public Task ImportFilteredAsync() => CanImportFiltered
        ? ImportBatchAsync(Filtered.Where(i => i.State != "imported" && !Results.ContainsKey(i.Path)).ToArray(), null) : Task.CompletedTask;
    public void StopAfterCurrent() { if (Importing) { StopRequested = true; Notify(); } }
    private async Task ImportBatchAsync(ExternalImportItem[] items, ExternalImportPreview? single)
    {
        if (items.Length == 0 || Closed || Importing || Uncertain) return;
        Importing = true; StopRequested = false; Error = null; Done = Imported = Updated = Skipped = Failed = 0; Total = items.Length; Notify();
        try
        {
            foreach (var item in items)
            {
                if (Closed || StopRequested) break;
                var writing = false;
                try
                {
                    var reviewed = single ?? await client.PreviewAsync(Project, Provider, Context, item, lifetime.Token);
                    if (Closed || StopRequested) break;
                    ValidatePreview(reviewed, item);
                    writing = true;
                    var result = await client.ImportAsync(reviewed);
                    if (Closed) break;
                    if (result.ProjectId != reviewed.ProjectId || result.Provider != reviewed.Provider || result.ContextId != reviewed.ContextId
                        || result.Path != reviewed.Path || result.SourceSessionId != reviewed.SourceSessionId || string.IsNullOrWhiteSpace(result.FrameId)
                        || result.MessageCount != reviewed.MessageCount || (reviewed.ExistingSessionId != null && reviewed.ExistingSessionId != result.FrameId)
                        || result.Status is not ("imported" or "updated" or "skipped")) throw new InvalidDataException("导入结果与确认的来源或目标不符。");
                    Results[item.Path] = result; ItemErrors.Remove(item.Path);
                    Items = Items.Select(i => i.Path == item.Path ? i with { State = "imported", MessageCount = result.MessageCount } : i).ToArray();
                    if (result.Status == "imported") Imported++; else if (result.Status == "updated") Updated++; else Skipped++;
                }
                catch (OperationCanceledException) when (!writing && Closed) { break; }
                catch (Exception ex)
                {
                    if (Closed) break;
                    Failed++; ItemErrors[item.Path] = ex.Message;
                    if (writing) { Uncertain = true; Error = "当前导入结果未能确认，已停止后续导入且不会重试。请先在目标项目核对。\n" + ex.Message; }
                }
                Done++; Notify();
                if (Uncertain) break;
            }
        }
        finally { Importing = false; if (!Closed) Notify(); }
    }
    private void Notify() { if (!Closed) Changed?.Invoke(); }
    public void Dispose() { Closed = true; StopRequested = true; selection++; listGeneration++; previewGeneration++; lifetime.Cancel(); listRead?.Cancel(); previewRead?.Cancel(); lifetime.Dispose(); }
}
