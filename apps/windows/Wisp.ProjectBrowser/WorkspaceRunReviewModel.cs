using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;
namespace Wisp.ProjectBrowser;

/// <summary>Explicitly scoped review. Reads may refresh; writes never replay.</summary>
public sealed class WorkspaceRunReviewModel(INativeRunReviewClient client, string project, string session)
{
    public event Action? Changed;
    public bool Visible { get; private set; }
    public string RunId { get; private set; } = "";
    public string Path { get; private set; } = "";
    public string Filter { get; set; } = "";
    public NativeRunWorkspaceEntry[] Entries { get; private set; } = [];
    public Dictionary<string, string> Selection { get; } = new(StringComparer.Ordinal);
    public bool Truncated { get; private set; }
    public bool Loading { get; private set; }
    public bool Mutating { get; private set; }
    public bool Ready { get; private set; }
    public bool ReadOnly { get; private set; } = true;
    public bool Cleaned { get; private set; }
    public string? Error { get; private set; }
    public string? Status { get; private set; }
    public string? DismissalError { get; private set; }
    public string? Confirmation { get; private set; }
    public string[] ConfirmedPaths { get; private set; } = [];
    public bool CanChange => Visible && Ready && !ReadOnly && !Cleaned && !Loading && !Mutating;
    private int generation, mutationRevision, offset;
    private readonly HashSet<string> dismissed = new(StringComparer.Ordinal);

    public Task OpenAsync(string runId, CancellationToken token = default, bool readOnly = true)
    {
        if (Visible) Close();
        generation++; Visible = true; RunId = runId; Path = ""; Filter = "";
        Entries = []; Selection.Clear(); Status = null; Error = null; DismissalError = null; Confirmation = null;
        ConfirmedPaths = []; Cleaned = false; Ready = false; ReadOnly = readOnly;
        return ReadAsync(false, token);
    }
    public void Close() => _ = CloseAsync();
    public async Task CloseAsync()
    {
        if (!Visible) return;
        var run = RunId;
        var persist = Visible && !ReadOnly && dismissed.Add(run);
        generation++; Visible = false; Loading = false; Ready = false; Confirmation = null; Selection.Clear();
        var current = generation;
        Changed?.Invoke();
        if (!persist) return;
        // Closing the view must not cancel this already authorized scoped write.
        // A lost acknowledgement is reported, never replayed on a later close.
        try { await client.InvokeAsync(project, session, run, new() { ["action"] = "dismiss" }); }
        catch (Exception ex)
        {
            if (current == generation) DismissalError = "运行 " + run + " 的审阅关闭状态未确认，不会自动重试。" + ex.Message;
        }
        Changed?.Invoke();
    }
    public bool HandleEscape()
    {
        if (!Visible) return false;
        if (Confirmation != null) CancelConfirmation(); else Close();
        return true;
    }
    public void CancelConfirmation() { Confirmation = null; ConfirmedPaths = []; Changed?.Invoke(); }
    public bool BeginConfirmation(string action)
    {
        if (!CanChange || Confirmation != null || action is not ("delete" or "cleanup") || action == "delete" && Selection.Count == 0) return false;
        Confirmation = action; ConfirmedPaths = Selection.Keys.Order(StringComparer.Ordinal).ToArray(); Changed?.Invoke(); return true;
    }
    public void Select(NativeRunWorkspaceEntry entry, bool selected)
    {
        if (!CanChange || Confirmation != null || entry.Kind is not ("file" or "dir") || !Entries.Contains(entry)) return;
        if (selected) Selection[entry.Path] = entry.Kind; else Selection.Remove(entry.Path);
    }
    public Task NavigateAsync(string path, CancellationToken token = default)
    {
        if (Mutating || Confirmation != null) return Task.CompletedTask;
        Path = path; Entries = []; return ReadAsync(false, token);
    }
    public string Parent => Path.Contains('/') ? Path[..Path.LastIndexOf('/')] : "";
    public async Task ReadAsync(bool append = false, CancellationToken token = default)
    {
        if (!Visible || Mutating || Confirmation != null || token.IsCancellationRequested || append && (!Ready || !Truncated || Loading)) return;
        var current = ++generation; var revision = mutationRevision; var run = RunId;
        Loading = true; Ready = false; Error = null;
        if (!append) { offset = 0; Entries = []; Truncated = false; }
        Changed?.Invoke();
        try
        {
            var reply = await client.InvokeAsync(project, session, run, new() { ["action"] = "list", ["path"] = Path, ["name_filter"] = Filter.Trim(), ["offset"] = append ? offset : 0 }, token);
            if (!Visible || current != generation || revision != mutationRevision || Mutating) return;
            var page = reply.Listing ?? throw new InvalidDataException("缺少运行文件列表。");
            if (append && page.Truncated && page.Entries.Length == 0) throw new InvalidDataException("运行文件分页没有推进，请刷新。");
            offset = append ? offset + page.Entries.Length : page.Entries.Length;
            Entries = (append ? Entries.Concat(page.Entries) : page.Entries).DistinctBy(e => e.Path, StringComparer.Ordinal).ToArray();
            Truncated = page.Truncated; ReadOnly = reply.ReadOnly; Cleaned = reply.Cleaned; Ready = true;
            if (Cleaned) { Selection.Clear(); Status = "服务器工作目录已清理。"; }
        }
        catch (Exception ex) { if (Visible && current == generation) Error = ex is OperationCanceledException ? "读取已取消。" : ex.Message; }
        finally { if (current == generation) Loading = false; Changed?.Invoke(); }
    }
    public Task DownloadAsync(CancellationToken token = default)
    {
        if (!CanChange || Confirmation != null || Selection.Count == 0) return Task.CompletedTask;
        return MutateAsync(new() { ["action"] = "download", ["files"] = Paths("file"), ["dirs"] = Paths("dir") }, token);
    }
    private JsonArray Paths(string kind) => new(Selection.Where(pair => pair.Value == kind).Select(pair => JsonValue.Create(pair.Key)).ToArray<JsonNode?>());
    public Task ConfirmAsync(CancellationToken token = default)
    {
        if (!CanChange || Confirmation == null || token.IsCancellationRequested) return Task.CompletedTask;
        var action = Confirmation; var paths = ConfirmedPaths.ToArray(); Confirmation = null; ConfirmedPaths = [];
        var operation = new JsonObject { ["action"] = action, ["confirmed"] = true };
        if (action == "delete") operation["paths"] = new JsonArray(paths.Select(path => JsonValue.Create(path)).ToArray<JsonNode?>());
        return MutateAsync(operation, token);
    }
    private async Task MutateAsync(JsonObject operation, CancellationToken token)
    {
        if (token.IsCancellationRequested) return;
        var current = generation; var run = RunId; Mutating = true; mutationRevision++;
        Error = null; Status = null; Changed?.Invoke();
        var refresh = false;
        try
        {
            var result = await client.InvokeAsync(project, session, run, operation, token);
            if (!Visible || current != generation) return;
            Selection.Clear(); Cleaned = result.Cleaned; Ready = false;
            Status = operation["action"]!.GetValue<string>() switch
            { "download" => $"已下载并登记 {result.Downloaded} 项产物。", "delete" => "已删除所选内容。", _ => "服务器工作目录已清理。" };
            refresh = true;
        }
        catch (Exception ex)
        {
            if (Visible && current == generation)
            { Ready = false; Error = "操作结果未确认，不会自动重试。请先刷新并核对结果。\n" + ex.Message; }
        }
        finally { mutationRevision++; Mutating = false; Changed?.Invoke(); }
        if (refresh && Visible && current == generation) await ReadAsync(false, token);
    }
}
