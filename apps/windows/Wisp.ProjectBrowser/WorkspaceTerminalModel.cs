using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Reconnecting reads may retry. Open and Write are never replayed after an ambiguous failure.
/// Hiding the panel must not call Close.</summary>
public sealed class WorkspaceTerminalModel
{
    private readonly INativeTerminalClient client;
    private readonly string projectId;
    private readonly string sessionId;
    public NativeTerminalInfo[] Terminals { get; private set; } = [];
    public NativePanelContext[] Contexts { get; private set; } = [];
    public string? SelectedId { get; private set; }
    public string Output { get; private set; } = "";
    public bool Busy { get; private set; }
    private readonly HashSet<string> uncertain = [];
    private readonly SemaphoreSlim writes = new(1, 1);
    private readonly SemaphoreSlim resizes = new(1, 1);
    private int resizeRevision;
    private System.Text.Decoder decoder = System.Text.Encoding.UTF8.GetDecoder();
    private bool detached;
    public event Action<NativeTerminalOutput>? OutputReceived;
    public int SelectionVersion => generation;
    public bool InputUncertain => SelectedId != null && uncertain.Contains(SelectedId);
    public uint? ExitCode { get; private set; }
    public string? Error { get; private set; }
    private ulong? cursor;
    private int generation;
    private readonly INativePanelClient? panels;
    public WorkspaceTerminalModel(INativeTerminalClient client, string projectId, string sessionId, INativePanelClient? panels = null)
    {
        this.client = client; this.projectId = projectId; this.sessionId = sessionId; this.panels = panels;
    }

    public void Select(string? id)
    {
        if (detached) return;
        generation++; cursor = null; SelectedId = id; ExitCode = null; Output = "";
        decoder.Reset(); Error = null;
    }

    public async Task ReloadAsync(CancellationToken cancellationToken = default, bool selectMissing = true)
    {
        if (detached) return;
        var current = generation;
        try
        {
            var rows = await client.ListAsync(projectId, sessionId, cancellationToken);
            if (detached || current != generation) return;
            if (rows.Any(row => row.ProjectId != projectId)) throw new InvalidDataException("Terminal project mismatch");
            Terminals = rows;
            if (selectMissing && (SelectedId == null || rows.All(row => row.Id != SelectedId))) Select(rows.FirstOrDefault()?.Id);
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!detached && current == generation) Error = ex.Message; }
    }

    public async Task LoadAsync(CancellationToken cancellationToken = default)
    {
        await ReloadAsync(cancellationToken);
        if (panels is null || detached) return;
        try { var contexts = await panels.ContextsAsync(projectId, sessionId, cancellationToken); if (!detached) Contexts = contexts.Attached; }
        catch (Exception ex) when (ex is not OperationCanceledException) { if (!detached) Error = ex.Message; }
    }

    public async Task OpenAsync(string contextId, CancellationToken cancellationToken = default)
    {
        if (Busy || detached) return;
        var current = generation;
        Busy = true; Error = null;
        try
        {
            var info = await client.OpenAsync(projectId, sessionId, contextId, cancellationToken);
            if (detached || current != generation) return;
            if (info.ProjectId != projectId) throw new InvalidDataException("Terminal project mismatch");
            await ReloadAsync(cancellationToken, false);
            if (!detached && generation == current) Select(info.Id);
        }
        catch (OperationCanceledException) { }
        catch (Exception ex)
        {
            if (detached || current != generation) return;
            await ReloadAsync(cancellationToken);
            if (!detached) Error = "终端创建未确认成功，请刷新列表后检查：" + ex.Message;
        }
        finally { Busy = false; }
    }

    public async Task ReadAsync(CancellationToken cancellationToken = default)
    {
        if (detached || SelectedId is not { } id) return;
        var current = generation;
        try
        {
            var chunk = await client.ReadAsync(projectId, sessionId, id, cursor, cancellationToken);
            if (detached || current != generation) return;
            var bytes = chunk.Bytes(id, cursor);
            if (chunk.Reset) { Output = ""; decoder.Reset(); }
            var chars = new char[System.Text.Encoding.UTF8.GetMaxCharCount(bytes.Length)];
            var count = decoder.GetChars(bytes, 0, bytes.Length, chars, 0, chunk.ExitCode != null);
            Output += new string(chars, 0, count);
            if (Output.Length > 1_048_576) Output = Output[^1_048_576..];
            cursor = chunk.End; ExitCode = chunk.ExitCode; Error = null;
            OutputReceived?.Invoke(chunk);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (current == generation) Error = ex.Message;
        }
    }

    public async Task WriteAsync(byte[] bytes, CancellationToken cancellationToken = default)
    {
        if (detached || SelectedId is not { } id || ExitCode != null || InputUncertain) return;
        var current = generation;
        try { await writes.WaitAsync(cancellationToken); }
        catch (OperationCanceledException) { return; }
        try
        {
            if (detached || current != generation || InputUncertain || ExitCode != null) return;
            try { await client.WriteAsync(projectId, sessionId, id, bytes, cancellationToken); }
            catch (Exception ex)
            {
                uncertain.Add(id);
                if (!detached && current == generation) Error = "输入未确认送达，不会自动重发：" + ex.Message;
            }
        }
        finally { writes.Release(); }
    }

    public void ResumeInput() { if (SelectedId != null) uncertain.Remove(SelectedId); Error = null; }

    public async Task ResizeAsync(ushort rows, ushort cols, CancellationToken token = default)
    {
        if (detached || SelectedId is not { } id || rows < 1 || cols < 2 || ExitCode != null) return;
        var current = generation;
        var revision = ++resizeRevision;
        try { await resizes.WaitAsync(token); }
        catch (OperationCanceledException) { return; }
        try
        {
            if (detached || current != generation || revision != resizeRevision) return;
            await client.ResizeAsync(projectId, sessionId, id, rows, cols, token);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (!detached && current == generation) Error = "终端尺寸更新失败：" + ex.Message; }
        finally { resizes.Release(); }
    }

    public async Task CloseSelectedAsync(CancellationToken cancellationToken = default)
    {
        if (detached || SelectedId is not { } id || Busy) return;
        var current = generation;
        Busy = true;
        try { await client.CloseAsync(projectId, sessionId, id, cancellationToken); if (!detached && current == generation) await ReloadAsync(cancellationToken); }
        catch (Exception ex) when (ex is not OperationCanceledException) { if (!detached && current == generation) Error = ex.Message; }
        finally { Busy = false; }
    }

    public void Detach() { detached = true; generation++; cursor = null; OutputReceived = null; }
}
