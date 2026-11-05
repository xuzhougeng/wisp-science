using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Session-scoped right panel. File mutations are not replayed after an uncertain response.</summary>
public sealed partial class WorkspacePanelModel
{
    private readonly INativePanelClient client;
    private readonly string projectId;
    private readonly string? sessionId;
    private string SessionId => sessionId ?? throw new InvalidOperationException("This action requires a conversation");
    public NativePanelTabs Tabs { get; }
    public NativePanelArtifact[] Artifacts { get; private set; } = [];
    public WorkspaceArtifactModel CreateArtifactPreview() => new(client, projectId, SessionId, Artifacts);
    public WorkspaceArtifactModel CreateGeneratedFilePreview(IEnumerable<NativeGeneratedOutput> outputs)
        => WorkspaceArtifactModel.ForGenerated(client, projectId, SessionId, outputs);
    public Task<NativePanelExport> ResolveArtifactFileAsync(string path, CancellationToken token = default)
        => client.ExportFileAsync(projectId, SessionId, path, token);
    public NativePanelFile[] Files { get; private set; } = [];
    public NativePanelContexts? Contexts { get; private set; }
    public string[] HostAliases { get; private set; } = [];
    public string? HostListError { get; private set; }
    public async Task<NativeRun[]> ReadContextRunsAsync(string context, CancellationToken token, bool explicitRefresh = false)
    {
        var settled = unverifiedContextRuns.ToDictionary();
        var rows = await client.ContextRunsAsync(projectId, sessionId, context, token);
        if (!closed && !token.IsCancellationRequested)
            foreach (var row in rows)
                if (settled.TryGetValue(row.Id, out var before) && unverifiedContextRuns.TryGetValue(row.Id, out var now)
                    && ReferenceEquals(before, now) && (now.Error == null || explicitRefresh)) unverifiedContextRuns.Remove(row.Id);
        return rows;
    }
    public Task<NativeRun> ReadContextRunDetailAsync(string context, string id, CancellationToken token) =>
        client.ContextRunDetailAsync(projectId, sessionId, context, id, token);
    public NativePanelFileContent? Preview { get; private set; }
    public NativeHighlight[] Highlights { get; private set; } = [];
    public NativeNotebookStar[] NotebookStars { get; private set; } = [];
    public NativeAgentSnapshot[] Agents { get; private set; } = [];
    public NativeAgentResult? AgentResult { get; private set; }
    public bool? DelegationEnabled { get; private set; }
    public bool DelegationBusy { get; private set; }
    public NativeContextActivity? Activity { get; private set; }
    private NativeRun[]? groupedRuns;
    private NativeSessionRunGroups runGroups = new([], []);
    public NativeSessionRunGroups SessionRuns
    {
        get
        {
            if (!ReferenceEquals(groupedRuns, Activity?.Runs))
            {
                groupedRuns = Activity?.Runs;
                runGroups = NativeSessionRunGroups.For(groupedRuns ?? [], sessionId);
            }
            return runGroups;
        }
    }
    public NativeRuntimeExecution? Execution { get; private set; }
    public NativeRun? RunDetail { get; private set; }
    public string? SelectedRunId { get; private set; }
    public bool RunLoading { get; private set; }
    public string? RunError { get; private set; }
    private int runRequest;
    private bool closed;
    public bool ActivityBusy { get; private set; }

    /// <summary>Run one guarded runtime mutation, then refresh the originating activity tab.
    /// Failures keep the previous state and are never retried automatically;
    /// an ambiguous stop/dismiss reports an explicit "check manually" error.</summary>
    private async Task PerformActivityAsync(Func<CancellationToken, Task> request, CancellationToken cancellationToken, bool ambiguousAllowed = false)
    {
        if (activity is null || ActivityBusy || closed || Activity?.ReadOnly != false || cancellationToken.IsCancellationRequested) return;
        var epoch = generation; activityRevision++; ActivityBusy = true; Error = null;
        try
        {
            try { await request(cancellationToken); }
            finally { activityRevision++; }
            if (epoch == generation && Tabs.Selected is "hosts" or "runs") await RefreshAsync(Tabs.Selected, cancellationToken: cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (epoch == generation) Error = (ambiguousAllowed ? "结果未确认，请刷新核对；不会自动重试。" : "操作未确认成功；不会自动重试。") + "\n" + ex.Message;
        }
        catch (OperationCanceledException)
        { if (epoch == generation) Error = "操作结果未确认，请重新读取核对；不会自动重试。"; }
        // This operation owns the busy flag. Its own refresh advances generation.
        finally { ActivityBusy = false; }
    }

    public Task StartRuntimeAsync(string contextId, string language, CancellationToken cancellationToken = default) =>
        PerformActivityAsync(token => activity!.StartRuntimeAsync(projectId, SessionId, contextId, language, token), cancellationToken);
    public Task StopRuntimeAsync(string runtimeId, ulong generation, CancellationToken cancellationToken = default) =>
        PerformActivityAsync(async token =>
        {
            if (await activity!.StopRuntimeAsync(projectId, SessionId, runtimeId, generation, token) is null)
                throw new InvalidOperationException("停止结果未知。");
        }, cancellationToken, ambiguousAllowed: true);
    public Task RestartRuntimeAsync(string runtimeId, ulong generation, CancellationToken cancellationToken = default) =>
        PerformActivityAsync(token => activity!.RestartRuntimeAsync(projectId, SessionId, runtimeId, generation, token), cancellationToken);
    public Task DismissRuntimeAsync(string runtimeId, ulong generation, CancellationToken cancellationToken = default) =>
        PerformActivityAsync(async token =>
        {
            await activity!.DismissRuntimeAsync(projectId, SessionId, runtimeId, generation, token);
        }, cancellationToken, ambiguousAllowed: true);
    public Task ExecuteAsync(string contextId, string language, string code, CancellationToken cancellationToken = default)
    {
        var epochValue = generation;
        return PerformActivityAsync(async token =>
        {
            var result = await activity!.ExecuteAsync(projectId, SessionId, contextId, language, code, token);
            if (epochValue == generation) Execution = result;
        }, cancellationToken);
    }
    public Task CancelRunAsync(string runId, CancellationToken cancellationToken = default)
    {
        var epochValue = generation;
        var selection = runRequest;
        return PerformActivityAsync(async token =>
        {
            var run = await activity!.CancelRunAsync(projectId, SessionId, runId, token);
            if (run is null || run.Id != runId) throw new InvalidOperationException("取消结果未知或运行身份不匹配。");
            if (epochValue == generation && selection == runRequest && SelectedRunId == runId) RunDetail = run;
        }, cancellationToken);
    }
    public Task HarvestRunAsync(string runId, CancellationToken cancellationToken = default)
    {
        var epochValue = generation;
        var selection = runRequest;
        return PerformActivityAsync(async token =>
        {
            var run = await activity!.HarvestRunAsync(projectId, SessionId, runId, token);
            if (run is null || run.Id != runId) throw new InvalidOperationException("收取结果未知或运行身份不匹配。");
            if (epochValue == generation && selection == runRequest && SelectedRunId == runId) RunDetail = run;
        }, cancellationToken);
    }
    public Task ReadRunAsync(string runId, CancellationToken cancellationToken = default) =>
        LoadRunAsync(runId, false, cancellationToken);
    public Task OpenRunAsync(string runId, CancellationToken cancellationToken = default) =>
        LoadRunAsync(runId, true, cancellationToken);
    private async Task LoadRunAsync(string runId, bool openRuns, CancellationToken cancellationToken)
    {
        if (activity is null || closed || sessionId == null || !Tabs.Available.Contains("runs")
            || !openRuns && Tabs.Selected != "runs" || string.IsNullOrWhiteSpace(runId) || cancellationToken.IsCancellationRequested) return;
        var request = ++runRequest;
        if (SelectedRunId != runId) RunDetail = null;
        SelectedRunId = runId; RunLoading = true; RunError = null;
        var current = generation;
        try
        {
            if (openRuns)
            {
                var refresh = RefreshAsync("runs", cancellationToken: cancellationToken);
                current = generation;
                await refresh;
            }
            if (closed || current != generation || request != runRequest || Tabs.Selected != "runs") return;
            var run = await activity.ReadRunAsync(projectId, SessionId, runId, cancellationToken);
            if (run.Id != runId) throw new InvalidDataException("运行详情身份不匹配，请重新读取。");
            if (!closed && current == generation && request == runRequest) { RunDetail = run; RunError = null; }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (!closed && current == generation && request == runRequest) RunError = ex.Message; }
        catch (OperationCanceledException)
        { if (!closed && current == generation && request == runRequest) RunError = "运行详情读取已取消。"; }
        finally { if (request == runRequest) RunLoading = false; }
    }

    public void DismissRun()
    { runRequest++; SelectedRunId = null; RunDetail = null; RunLoading = false; RunError = null; }

    public bool PreviewEditable { get; private set; }
    public string Path { get; private set; } = ".";
    public bool Loading { get; private set; }
    public bool FileActionBusy { get; private set; }
    public bool ContextBusy { get; private set; }
    public string? Error { get; private set; }
    public string Parent => RemoteFiles ? RemoteParent(Path) : ParentPath(Path);
    public static string Child(string directory, string name) => directory is "." or "" ? name : directory.TrimEnd('/') + "/" + name;
    public static string ParentPath(string path)
    {
        var trimmed = path.Replace('\\', '/').Trim().TrimEnd('/');
        var slash = trimmed.LastIndexOf('/');
        return slash <= 0 ? "." : trimmed[..slash];
    }
    private int generation;
    private string lastReadTab;
    private readonly INativeHighlightClient? highlights;
    private readonly INativeNotebookClient? notebook;
    private readonly INativeAgentPanelClient? agents;
    private readonly INativeContextActivityClient? activity;

    public WorkspacePanelModel(INativePanelClient client, string projectId, string? sessionId, NativePanelTabs tabs,
        INativeHighlightClient? highlights = null, INativeNotebookClient? notebook = null, INativeAgentPanelClient? agents = null,
        INativeContextActivityClient? activity = null, INativeFilesClient? files = null)
    {
        this.client = client; this.projectId = projectId; this.sessionId = sessionId; Tabs = tabs; lastReadTab = tabs.Selected;
        this.highlights = highlights; this.notebook = notebook; this.agents = agents; this.activity = activity;
        fileClient = files;
    }

    public async Task RefreshAsync(string tab, string? directory = null, CancellationToken cancellationToken = default)
    {
        if (closed || tab != NativePanelTabs.Home && !Tabs.Available.Contains(tab)
            || sessionId == null && tab is not (NativePanelTabs.Home or "files" or "memos" or "runs") && !(tab == "hosts" && Tabs.Available.Contains("hosts"))) return;
        if (tab != "runs") { runRequest++; RunLoading = false; }
        var contextReadRevision = contextRevision;
        var activityReadRevision = activityRevision;
        var contextReadDuringWrite = ContextBusy;
        // Closing the selected tab updates Tabs.Selected before the replacement is read.
        var changingTab = lastReadTab != tab;
        lastReadTab = tab;
        if (changingTab) ClearFileSelection(leaveMode: true);
        if (changingTab || directory != null) ClearFileSearch();
        Tabs.Show(tab);
        if (changingTab || tab == "files" && directory != null && directory != Path) { Preview = null; PreviewEditable = false; }
        var current = ++generation;
        Loading = true; Error = null;
        var requested = directory ?? Path;
        try
        {
            if (tab is NativePanelTabs.Home or "provenance" or "sidechat" or "memos") return;
            if (tab == "runs" && sessionId == null) return;
            if (tab == "artifacts")
            {
                var result = await client.ArtifactsAsync(projectId, SessionId, cancellationToken);
                if (current == generation) Artifacts = result;
            }
            else if (tab == "runs" && activity is not null)
            {
                // Runs do not depend on context/runtime discovery succeeding.
                var result = await ReadActivityAsync(cancellationToken);
                if (current == generation && activityReadRevision == activityRevision) { Activity = result; RunRefreshError = null; }
            }
            else if (tab == "hosts")
            {
                var contexts = sessionId == null ? await client.ProjectContextsAsync(projectId, cancellationToken)
                    : await client.ContextsAsync(projectId, SessionId, cancellationToken);
                if (current != generation || contextReadDuringWrite || contextReadRevision != contextRevision || ContextBusy) return;
                string[] aliases = []; string? aliasError = null;
                try { aliases = await client.SshAliasesAsync(projectId, cancellationToken); }
                catch (Exception ex) when (ex is not OperationCanceledException) { aliasError = ex.Message; }
                if (current != generation || contextReadRevision != contextRevision || ContextBusy) return;
                Contexts = contexts; HostAliases = aliases; HostListError = aliasError; ContextUncertain = false;
                if (activity is not null && sessionId != null)
                {
                    var result = await ReadActivityAsync(cancellationToken);
                    if (current != generation || activityReadRevision != activityRevision || contextReadRevision != contextRevision || ContextBusy) return;
                    Activity = result;
                }
            }
            else if (tab == "highlights" && highlights is not null)
            {
                var result = await highlights.ListAsync(projectId, SessionId, cancellationToken);
                if (current == generation) Highlights = result;
            }
            else if (tab == "notebook" && notebook is not null)
            {
                var result = await notebook.ListStarsAsync(projectId, SessionId, cancellationToken);
                if (current == generation) NotebookStars = result;
            }
            else if (tab == "agents" && agents is not null)
            {
                var result = await agents.ListAsync(projectId, SessionId, cancellationToken);
                if (current != generation) return;
                var enabled = await agents.GetDelegationAsync(projectId, SessionId, cancellationToken);
                if (current != generation) return;
                Agents = result;
                DelegationEnabled = enabled;
            }
            else if (tab == "files")
            {
                if (fileClient != null) { await RefreshFilesAsync(current, requested, cancellationToken); return; }
                var result = await client.FilesAsync(projectId, SessionId, requested, cancellationToken);
                if (current != generation) return;
                Files = result;
                Path = requested;
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception ex)
        {
            if (current == generation)
            {
                if (tab == "runs") { if (activityReadRevision == activityRevision) RunRefreshError = ex.Message; }
                else Error = ex.Message;
            }
        }
        finally { if (current == generation) Loading = false; }
    }

    public async Task PerformFileActionAsync(NativePanelFileAction action, string path, string? newPath = null,
        CancellationToken cancellationToken = default)
    {
        if (!CanMutateLocalFiles || cancellationToken.IsCancellationRequested) return;
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
        finally { FileActionBusy = false; }
    }

    public async Task ReadFileAsync(string path, CancellationToken cancellationToken = default)
    {
        var current = ++generation;
        if (closed) return;
        var canEdit = !RemoteFiles && (fileClient == null || FileLocations is { ReadOnly: false });
        try {
            var value = fileClient == null ? await client.ReadFileAsync(projectId, SessionId, path, cancellationToken)
                : await fileClient.ReadAsync(projectId, sessionId, FileContext, path, cancellationToken);
            if (current == generation) { Preview = value; CanEditPreview = canEdit; PreviewEditable = false; Error = null; }
        }
        catch (Exception ex) when (ex is not OperationCanceledException) { if (current == generation) Error = ex.Message; }
    }

    public async Task<NativePanelFileContent> ReadGeneratedOutputAsync(NativeGeneratedOutput output, CancellationToken token)
    {
        if (closed || output.ProjectId != projectId || output.SessionId != sessionId || output.Path == null || output.Superseded)
            throw new InvalidOperationException("Generated output no longer belongs to the open conversation");
        var value = fileClient == null ? await client.ReadFileAsync(projectId, SessionId, output.Path, token)
            : await fileClient.ReadAsync(projectId, sessionId, "local", output.Path, token);
        if (closed) throw new OperationCanceledException();
        return value;
    }

    public async Task<NativePanelFileContent> ReadRegisteredOutputAsync(NativePanelArtifact artifact, CancellationToken token)
    {
        if (closed || !Artifacts.Any(item => item.Id == artifact.Id))
            throw new InvalidOperationException("Artifact no longer belongs to the open conversation");
        var value = await client.ReadArtifactAsync(projectId, SessionId, artifact.Id, token);
        if (closed) throw new OperationCanceledException();
        return value;
    }

    public async Task ReadArtifactAsync(string artifactId, CancellationToken cancellationToken = default)
    {
        var current = ++generation;
        try { var value = await client.ReadArtifactAsync(projectId, SessionId, artifactId, cancellationToken); if (current == generation) { Preview = value; CanEditPreview = true; PreviewEditable = false; Error = null; } }
        catch (Exception ex) when (ex is not OperationCanceledException) { if (current == generation) Error = ex.Message; }
    }

    public async Task SaveFileAsync(string originalText, string text, CancellationToken cancellationToken = default)
    {
        if (closed || Preview is null || !CanEditPreview) return;
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
            if (!await highlights.RemoveAsync(projectId, SessionId, id, cancellationToken))
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
                if (!await notebook.UnstarAsync(projectId, SessionId, saved.Id, cancellationToken))
                    throw new InvalidOperationException("收藏已发生变化，请刷新后重试。");
                if (epoch == generation) NotebookStars = NotebookStars.Where(row => row.Id != saved.Id).ToArray();
            }
            else
            {
                var row = await notebook.StarAsync(projectId, SessionId, cell.Language, cell.Source, cancellationToken);
                if (epoch == generation) NotebookStars = NotebookStars.Where(item => item.Id != row.Id).Append(row).ToArray();
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (epoch == generation) Error = ex.Message; }
    }

    public async Task SetContextEnabledAsync(string contextId, bool enabled, CancellationToken cancellationToken = default)
    {
        if (!CanChangeContext || cancellationToken.IsCancellationRequested) return;
        await ChangeContextAsync(async token =>
        {
            var ids = await client.SetContextEnabledAsync(projectId, SessionId, contextId, enabled, token);
            if (ids.Contains(contextId) != enabled) throw new InvalidDataException("Environment attachment was not confirmed.");
        }, cancellationToken);
    }

    public Task ProbeAttachedAsync(string id, CancellationToken token = default) =>
        CanManageHosts && Contexts?.Attached.Any(c => c.Id == id) == true && !token.IsCancellationRequested
        ? ChangeContextAsync(ct => client.ProbeContextAsync(projectId, id, ct), token) : Task.CompletedTask;

    public bool ContextUncertain { get; private set; }
    private int contextRevision;
    public bool CanManageHosts => !closed && !ContextBusy && !ContextUncertain && !Loading && Contexts is { ReadOnly: false };
    public bool CanChangeContext => sessionId != null && CanManageHosts;
    public Task SetDefaultContextAsync(string contextId, CancellationToken cancellationToken = default)
    {
        if (!CanChangeContext || Contexts?.DefaultContext is null || cancellationToken.IsCancellationRequested
            || Contexts.DefaultContext.ContextId == contextId
            || !Contexts.Contexts.Any(context => context.Id == contextId || contextId == "local" && context.Kind == "local"))
            return Task.CompletedTask;
        return ChangeContextAsync(token => client.SetDefaultContextAsync(projectId, SessionId, contextId, token), cancellationToken);
    }

    private async Task ChangeContextAsync(Func<CancellationToken, Task> action, CancellationToken cancellationToken)
    {
        var current = generation; activityRevision++; ContextBusy = true; ContextUncertain = true; contextRevision++;
        var confirmed = false;
        try
        {
            await action(cancellationToken);
            confirmed = true;
        }
        catch (Exception ex)
        {
            ContextUncertain = true;
            if (generation == current) Error = "环境设置结果未确认，请刷新核对；不会自动重试。\n" + ex.Message;
        }
        finally { contextRevision++; activityRevision++; ContextBusy = false; }
        if (confirmed && generation == current) await RefreshAsync("hosts", cancellationToken: cancellationToken);
    }

    public async Task SetDelegationAsync(bool enabled, CancellationToken cancellationToken = default)
    {
        if (agents is null || DelegationBusy) return;
        var epoch = generation; DelegationBusy = true;
        try
        {
            if (!await agents.SetDelegationAsync(projectId, SessionId, enabled, cancellationToken))
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
            await agents.ActAsync(projectId, SessionId, snapshot.Workflow.Id, action,
                action == NativeAgentAction.Approve ? snapshot.Workflow.Version : null, null, cancellationToken);
            if (epoch == generation && Tabs.Selected == "agents") await RefreshAsync("agents", cancellationToken: cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (epoch == generation) Error = "操作结果未确认，未自动重试。" + ex.Message; }
    }

    public bool CanEditPreview { get; private set; } = true;
    public void EditPreview() { PreviewEditable = CanEditPreview && Preview?.Text != null; }
    public void DismissPreview() { generation++; Preview = null; PreviewEditable = false; AgentResult = null; }
    public void Close() { closed = true; generation++; DismissRun(); Loading = false; FileActionBusy = false; ContextBusy = false; Preview = null; }
}
