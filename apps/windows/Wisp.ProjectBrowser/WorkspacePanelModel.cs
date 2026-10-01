using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Session-scoped right panel. File mutations are not replayed after an uncertain response.</summary>
public sealed class WorkspacePanelModel
{
    private readonly INativePanelClient client;
    private readonly string projectId;
    private readonly string sessionId;
    public NativePanelTabs Tabs { get; }
    public NativePanelArtifact[] Artifacts { get; private set; } = [];
    public NativePanelFile[] Files { get; private set; } = [];
    public NativePanelContexts? Contexts { get; private set; }
    public NativePanelFileContent? Preview { get; private set; }
    public NativeHighlight[] Highlights { get; private set; } = [];
    public NativeNotebookStar[] NotebookStars { get; private set; } = [];
    public NativeAgentSnapshot[] Agents { get; private set; } = [];
    public NativeAgentResult? AgentResult { get; private set; }
    public bool? DelegationEnabled { get; private set; }
    public bool DelegationBusy { get; private set; }
    public NativeContextActivity? Activity { get; private set; }
    public NativeRuntimeExecution? Execution { get; private set; }
    public NativeRun? RunDetail { get; private set; }
    public bool ActivityBusy { get; private set; }

    /// <summary>Run one guarded runtime mutation, then refresh the hosts tab.
    /// Failures keep the previous state and are never retried automatically;
    /// an ambiguous stop/dismiss reports an explicit "check manually" error.</summary>
    private async Task PerformActivityAsync(Func<CancellationToken, Task> request, CancellationToken cancellationToken, bool ambiguousAllowed = false)
    {
        if (activity is null || ActivityBusy) return;
        var epoch = generation; ActivityBusy = true; Error = null;
        try
        {
            await request(cancellationToken);
            if (epoch == generation && Tabs.Selected == "hosts") await RefreshAsync("hosts", cancellationToken: cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (epoch == generation) Error = (ambiguousAllowed ? "结果未确认，请刷新核对；不会自动重试。" : "操作未确认成功；不会自动重试。") + "\n" + ex.Message;
        }
        finally { if (epoch == generation) ActivityBusy = false; }
    }

    public Task StartRuntimeAsync(string contextId, string language, CancellationToken cancellationToken = default) =>
        PerformActivityAsync(token => activity!.StartRuntimeAsync(projectId, sessionId, contextId, language, token), cancellationToken);
    public Task StopRuntimeAsync(string runtimeId, ulong generation, CancellationToken cancellationToken = default) =>
        PerformActivityAsync(async token =>
        {
            if (await activity!.StopRuntimeAsync(projectId, sessionId, runtimeId, generation, token) is null)
                throw new InvalidOperationException("停止结果未知。");
        }, cancellationToken, ambiguousAllowed: true);
    public Task RestartRuntimeAsync(string runtimeId, ulong generation, CancellationToken cancellationToken = default) =>
        PerformActivityAsync(token => activity!.RestartRuntimeAsync(projectId, sessionId, runtimeId, generation, token), cancellationToken);
    public Task DismissRuntimeAsync(string runtimeId, ulong generation, CancellationToken cancellationToken = default) =>
        PerformActivityAsync(async token =>
        {
            await activity!.DismissRuntimeAsync(projectId, sessionId, runtimeId, generation, token);
        }, cancellationToken, ambiguousAllowed: true);
    public Task ExecuteAsync(string contextId, string language, string code, CancellationToken cancellationToken = default)
    {
        var epochValue = generation;
        return PerformActivityAsync(async token =>
        {
            var result = await activity!.ExecuteAsync(projectId, sessionId, contextId, language, code, token);
            if (epochValue == generation) Execution = result;
        }, cancellationToken);
    }
    public Task CancelRunAsync(string runId, CancellationToken cancellationToken = default) =>
        PerformActivityAsync(async token =>
        {
            var run = await activity!.CancelRunAsync(projectId, sessionId, runId, token);
            if (run is null) throw new InvalidOperationException("取消结果未知。");
        }, cancellationToken);
    public Task HarvestRunAsync(string runId, CancellationToken cancellationToken = default)
    {
        var epochValue = generation;
        return PerformActivityAsync(async token =>
        {
            var run = await activity!.HarvestRunAsync(projectId, sessionId, runId, token);
            if (run is null) throw new InvalidOperationException("收取结果未知。");
            if (epochValue == generation) RunDetail = run;
        }, cancellationToken);
    }
    public async Task ReadRunAsync(string runId, CancellationToken cancellationToken = default)
    {
        if (activity is null) return;
        var current = generation;
        try
        {
            var run = await activity.ReadRunAsync(projectId, sessionId, runId, cancellationToken);
            if (current == generation) { RunDetail = run; Error = null; }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (current == generation) Error = ex.Message; }
    }

    public bool PreviewEditable { get; private set; }
    public string Path { get; private set; } = ".";
    public bool Loading { get; private set; }
    public bool FileActionBusy { get; private set; }
    public bool ContextBusy { get; private set; }
    public string? Error { get; private set; }
    public string Parent => ParentPath(Path);
    public static string Child(string directory, string name) => directory is "." or "" ? name : directory.TrimEnd('/') + "/" + name;
    public static string ParentPath(string path)
    {
        var trimmed = path.Replace('\\', '/').Trim().TrimEnd('/');
        var slash = trimmed.LastIndexOf('/');
        return slash <= 0 ? "." : trimmed[..slash];
    }
    private int generation;
    private readonly INativeHighlightClient? highlights;
    private readonly INativeNotebookClient? notebook;
    private readonly INativeAgentPanelClient? agents;
    private readonly INativeContextActivityClient? activity;

    public WorkspacePanelModel(INativePanelClient client, string projectId, string sessionId, NativePanelTabs tabs,
        INativeHighlightClient? highlights = null, INativeNotebookClient? notebook = null, INativeAgentPanelClient? agents = null,
        INativeContextActivityClient? activity = null)
    {
        this.client = client; this.projectId = projectId; this.sessionId = sessionId; Tabs = tabs;
        this.highlights = highlights; this.notebook = notebook; this.agents = agents; this.activity = activity;
    }

    public async Task RefreshAsync(string tab, string? directory = null, CancellationToken cancellationToken = default)
    {
        Tabs.Show(tab);
        var current = ++generation;
        Loading = true; Error = null;
        var requested = directory ?? Path;
        try
        {
            if (tab is "provenance" or "sidechat") return;
            if (tab == "artifacts") Artifacts = await client.ArtifactsAsync(projectId, sessionId, cancellationToken);
            else if (tab == "hosts")
            {
                Contexts = await client.ContextsAsync(projectId, sessionId, cancellationToken);
                if (activity is not null) Activity = await activity.ReadAsync(projectId, sessionId, cancellationToken);
            }
            else if (tab == "highlights" && highlights is not null) Highlights = await highlights.ListAsync(projectId, sessionId, cancellationToken);
            else if (tab == "notebook" && notebook is not null) NotebookStars = await notebook.ListStarsAsync(projectId, sessionId, cancellationToken);
            else if (tab == "agents" && agents is not null)
            {
                Agents = await agents.ListAsync(projectId, sessionId, cancellationToken);
                DelegationEnabled = await agents.GetDelegationAsync(projectId, sessionId, cancellationToken);
            }
            else if (tab == "files")
            {
                Files = await client.FilesAsync(projectId, sessionId, requested, cancellationToken);
                Path = requested;
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (current == generation) Error = ex.Message; }
        finally { if (current == generation) Loading = false; }
    }

    public async Task PerformFileActionAsync(NativePanelFileAction action, string path, string? newPath = null,
        CancellationToken cancellationToken = default)
    {
        if (FileActionBusy) throw new InvalidOperationException("文件操作正在进行。");
        var epoch = generation; var directory = Path;
        FileActionBusy = true;
        try
        {
            await client.FileActionAsync(projectId, sessionId, action, path, newPath, cancellationToken);
            if (epoch == generation && Tabs.Selected == "files" && Path == directory)
                await RefreshAsync("files", directory, cancellationToken);
        }
        catch (Exception)
        {
            if (epoch == generation) throw;
        }
        finally { if (epoch == generation) FileActionBusy = false; }
    }

    public async Task ReadFileAsync(string path, CancellationToken cancellationToken = default)
    {
        var current = ++generation;
        try { Preview = await client.ReadFileAsync(projectId, sessionId, path, cancellationToken); }
        catch (Exception ex) when (ex is not OperationCanceledException) { if (current == generation) Error = ex.Message; }
    }

    public async Task ReadArtifactAsync(string artifactId, CancellationToken cancellationToken = default)
    {
        var current = ++generation;
        try { Preview = await client.ReadArtifactAsync(projectId, sessionId, artifactId, cancellationToken); }
        catch (Exception ex) when (ex is not OperationCanceledException) { if (current == generation) Error = ex.Message; }
    }

    public async Task SaveFileAsync(string originalText, string text, CancellationToken cancellationToken = default)
    {
        if (Preview is null) return;
        var current = generation; var path = Preview.Path;
        await client.SaveFileAsync(projectId, sessionId, path, originalText, text, cancellationToken);
        if (current != generation || Preview?.Path != path) return;
        Preview = Preview with { Text = text, TotalBytes = (ulong)System.Text.Encoding.UTF8.GetByteCount(text) };
        PreviewEditable = false;
    }

    public async Task RemoveHighlightAsync(string id, CancellationToken cancellationToken = default)
    {
        if (highlights is null) return;
        var epoch = generation;
        try
        {
            if (!await highlights.RemoveAsync(projectId, sessionId, id, cancellationToken))
                throw new InvalidOperationException("摘录已发生变化，请刷新后重试。");
            if (epoch == generation) Highlights = Highlights.Where(row => row.Id != id).ToArray();
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (epoch == generation) Error = ex.Message; }
    }

    public async Task ToggleNotebookStarAsync(NativeNotebookCell cell, CancellationToken cancellationToken = default)
    {
        if (notebook is null) return;
        var epoch = generation;
        try
        {
            var saved = NotebookStars.FirstOrDefault(row => row.Matches(cell));
            if (saved is not null)
            {
                if (!await notebook.UnstarAsync(projectId, sessionId, saved.Id, cancellationToken))
                    throw new InvalidOperationException("收藏已发生变化，请刷新后重试。");
                if (epoch == generation) NotebookStars = NotebookStars.Where(row => row.Id != saved.Id).ToArray();
            }
            else
            {
                var row = await notebook.StarAsync(projectId, sessionId, cell.Language, cell.Source, cancellationToken);
                if (epoch == generation) NotebookStars = NotebookStars.Where(item => item.Id != row.Id).Append(row).ToArray();
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (epoch == generation) Error = ex.Message; }
    }

    public async Task SetContextEnabledAsync(string contextId, bool enabled, CancellationToken cancellationToken = default)
    {
        if (ContextBusy || Contexts?.ReadOnly == true) return;
        var current = generation; ContextBusy = true;
        try
        {
            await client.SetContextEnabledAsync(projectId, sessionId, contextId, enabled, cancellationToken);
            if (generation == current) await RefreshAsync("hosts", cancellationToken: cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (generation == current) Error = ex.Message; }
        finally { if (generation == current) ContextBusy = false; }
    }

    public async Task SetDelegationAsync(bool enabled, CancellationToken cancellationToken = default)
    {
        if (agents is null || DelegationBusy) return;
        var epoch = generation; DelegationBusy = true;
        try
        {
            if (!await agents.SetDelegationAsync(projectId, sessionId, enabled, cancellationToken))
                throw new InvalidOperationException("委派设置未确认。");
            if (epoch == generation) DelegationEnabled = enabled;
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (epoch == generation) Error = "委派设置未确认，未自动重试。请刷新后重试。" + ex.Message;
        }
        finally { if (epoch == generation) DelegationBusy = false; }
    }

    public async Task ActAgentAsync(NativeAgentSnapshot snapshot, NativeAgentAction action, CancellationToken cancellationToken = default)
    {
        if (agents is null || snapshot.Workflow.Depth != 0) return;
        var epoch = generation;
        try
        {
            await agents.ActAsync(projectId, sessionId, snapshot.Workflow.Id, action,
                action == NativeAgentAction.Approve ? snapshot.Workflow.Version : null, null, cancellationToken);
            if (epoch == generation && Tabs.Selected == "agents") await RefreshAsync("agents", cancellationToken: cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (epoch == generation) Error = "操作结果未确认，未自动重试。" + ex.Message; }
    }

    public void EditPreview() { PreviewEditable = Preview?.Text != null; }
    public void DismissPreview() { generation++; Preview = null; PreviewEditable = false; AgentResult = null; }
    public void Close() { generation++; Loading = false; FileActionBusy = false; ContextBusy = false; Preview = null; }
}
