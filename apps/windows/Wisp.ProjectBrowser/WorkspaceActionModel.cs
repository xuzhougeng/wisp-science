using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>One mounted native surface. Closing invalidates replies; mutations are never replayed.</summary>
public class WorkspaceActionModel : IDisposable
{
    public event Action? Changed;
    public bool Busy { get; private set; }
    public bool Closed { get; private set; }
    public string? Error { get; private set; }
    private int generation;
    public void Notify() => Changed?.Invoke();
    public void Fail(string message) { Error = message; Notify(); }
    public async Task<bool> RunAsync<T>(Func<Task<T>> request, Action<T> apply)
    {
        if (Busy || Closed) return false;
        var current = ++generation;
        Busy = true; Error = null; Notify();
        try
        {
            var value = await request();
            if (Closed || generation != current) return false;
            apply(value);
            return true;
        }
        catch (Exception ex)
        {
            if (!Closed && generation == current) Error = "操作结果未能确认；不会自动重试。\n" + ex.Message;
            return false;
        }
        finally { if (generation == current) { Busy = false; Notify(); } }
    }
    public virtual void Dispose() { Closed = true; generation++; Busy = false; }
}

public sealed record ProjectFolder(string Id, string Name);
public sealed record SessionSection(string Title, string? FolderId, BrowserSession[] Sessions);

public sealed class WorkspaceSessionGroups(INativeSettingsClient client, string projectId) : WorkspaceActionModel
{
    public ProjectFolder[] Folders { get; private set; } = [];
    public string Sort { get; set; } = "newest";
    public string Group { get; set; } = "none";
    public bool Selecting { get; set; }
    public HashSet<string> Selected { get; } = [];
    public string Draft { get; set; } = "";
    public string? RenamingId { get; set; }
    public Task<bool> LoadAsync() => RunAsync(
        async () => (await client.InvokeAsync("native_project_folders", new(), projectId))?.Deserialize<ProjectFolder[]>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing project folders"), value => Folders = value);

    public async Task<bool> SaveAsync()
    {
        if (string.IsNullOrWhiteSpace(Draft)) { Fail("请填写分组名称。"); return false; }
        var args = new JsonObject { ["name"] = Draft.Trim() };
        var rename = RenamingId;
        if (rename != null) args["folder_id"] = rename;
        var saved = await RunAsync(() => client.InvokeAsync(rename == null ? "native_project_folder_create" : "native_project_folder_rename", args, projectId), _ => { Draft = ""; RenamingId = null; });
        if (saved) await LoadAsync();
        return saved;
    }
    public Task<bool> MoveAsync(string? folderId) => RunAsync(async () =>
    {
        foreach (var id in Selected.ToArray())
        {
            await client.InvokeAsync("native_project_session_move", new() { ["session_id"] = id, ["folder_id"] = folderId }, projectId);
            // Do not replay earlier successes if a later move fails.
            Selected.Remove(id);
        }
        return true;
    }, _ => Selecting = false);

    public SessionSection[] Sections(IEnumerable<BrowserSession> sessions)
    {
        var ordered = Sort == "name" ? sessions.OrderBy(s => s.Title, StringComparer.CurrentCultureIgnoreCase).ThenBy(s => s.Id).ToArray()
            : sessions.OrderByDescending(s => s.Timestamp).ThenBy(s => s.Id).ToArray();
        if (Group == "date") return ordered.GroupBy(s => DateTimeOffset.FromUnixTimeSeconds(s.Timestamp).LocalDateTime.ToString("yyyy-MM-dd"))
            .Select(g => new SessionSection(g.Key, null, g.ToArray())).ToArray();
        if (Group != "folder") return [new("会话", null, ordered)];
        return Folders.Select(f => new SessionSection(f.Name, f.Id, ordered.Where(s => s.FolderId == f.Id).ToArray()))
            .Append(new("未分组", null, ordered.Where(s => !Folders.Any(f => f.Id == s.FolderId)).ToArray())).ToArray();
    }
}

public sealed class WorkspaceLibraryModel(INativeLibraryClient client) : WorkspaceActionModel
{
    public string Query { get; set; } = "";
    public string? Kind { get; set; }
    public IReadOnlyList<LibraryItemSummary> Items { get; private set; } = [];
    public Task<bool> SearchAsync() => RunAsync(() => client.SearchAsync(Query, Kind), rows => Items = rows);
    public Task<bool> DeleteAsync(string id) => RunAsync(async () =>
    {
        if (!await client.DeleteAsync(id)) throw new InvalidOperationException("这条收藏已不存在，请刷新。");
        return id;
    }, removed => Items = Items.Where(i => i.Id != removed).ToArray());
    public static string ComposerText(LibraryItemSummary item) => string.IsNullOrWhiteSpace(item.CodePreview)
        ? $"请看收藏「{item.Title}」。" : $"请用收藏「{item.Title}」重新运行：\n```{item.Language}\n{item.CodePreview.Trim()}\n```";
}

public sealed class WorkspaceProjectCreation(INativeProjectClient client) : WorkspaceActionModel
{
    public const string StandardContext = """
        本项目采用标准科研目录结构。产物请写入以下路径（相对项目根目录）：
        - 图表 -> figures/
        - 表格 -> results/tables/
        - 拟合模型 -> results/models/
        - 报告 -> results/reports/
        - 脚本与 notebook -> analysis/scripts/、analysis/notebooks/
        - 数据 -> data/raw/（原始输入，不修改）、data/processed/（衍生数据）
        - 从远程主机拉取的文件 -> remote/<服务器>/，每台服务器一个子目录，用该执行上下文的名称命名
        - 文献与 PDF -> literature/
        不要把生成的文件留在项目根目录。
        """;
    public string Name { get; set; } = "";
    public string Directory { get; set; } = "";
    public string Description { get; set; } = "";
    public string AgentContext { get; set; } = "";
    public bool StandardLayout { get; set; }
    public void SetStandardLayout(bool enabled)
    {
        StandardLayout = enabled;
        var rest = AgentContext.Replace(StandardContext, "", StringComparison.Ordinal).Trim();
        AgentContext = enabled ? (rest.Length == 0 ? StandardContext : rest + "\n\n" + StandardContext) : rest;
    }
    public ProjectSummary? Created { get; private set; }
    public Task<bool> CreateAsync()
    {
        if (string.IsNullOrWhiteSpace(Name) || string.IsNullOrWhiteSpace(Directory))
        { Fail("请填写项目名称和工作目录。"); return Task.FromResult(false); }
        return RunAsync(() => client.CreateAsync(Name.Trim(), Directory.Trim(), Description, AgentContext, StandardLayout), row => Created = row);
    }
    public Task<bool> ImportAsync(string path) => RunAsync(() => client.ImportAsync(path), row => Created = row);
}

public sealed class WorkspacePublicationModel(INativePublicationClient client, string projectId) : WorkspaceActionModel
{
    public string Title { get; set; } = "";
    public string Description { get; set; } = "";
    public string RevisionLabel { get; set; } = "";
    public NativePublicationWorkspace? Workspace { get; private set; }
    public bool CreationAvailable => Workspace is { Publications.Count: 0 };
    public bool CanCreate => CreationAvailable && !Busy && !Closed
        && !string.IsNullOrWhiteSpace(Title) && !string.IsNullOrWhiteSpace(RevisionLabel);
    public Task<bool> LoadAsync() => RunAsync(() => client.ReadAsync(projectId), value => Workspace = value);
    public Task<bool> CreateAsync()
    {
        if (!CreationAvailable || Busy || Closed) return Task.FromResult(false);
        if (string.IsNullOrWhiteSpace(Title) || string.IsNullOrWhiteSpace(RevisionLabel))
        { Fail("请填写论文标题和版本标签。"); return Task.FromResult(false); }
        return RunAsync(() => client.CreateAsync(projectId, Title.Trim(), Description, RevisionLabel.Trim()), value =>
        { Workspace = value; Title = ""; Description = ""; RevisionLabel = ""; });
    }
}
