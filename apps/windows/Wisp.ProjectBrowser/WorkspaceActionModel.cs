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
    protected void ClearFailure() => Error = null;
    protected virtual string UnconfirmedFailure(Exception exception) => "操作结果未能确认；不会自动重试。\n" + exception.Message;
    public async Task<bool> RunAsync<T>(Func<Task<T>> request, Action<T> apply, Func<Exception, string>? describeFailure = null)
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
            if (!Closed && generation == current) Error = (describeFailure ?? UnconfirmedFailure)(ex);
            return false;
        }
        finally { if (generation == current) { Busy = false; Notify(); } }
    }
    public virtual void Dispose() { Closed = true; generation++; Busy = false; }
}

public sealed record ProjectFolder(string Id, string Name);
public sealed record SessionSection(string Title, string? FolderId, BrowserSession[] Sessions, string? LabelKey = null)
{
    public string DisplayTitle(NativeLocalization locale) => LabelKey != null ? locale.Text(LabelKey)
        : string.IsNullOrWhiteSpace(Title) ? locale.Text("folder.untitled") : Title;
}

public sealed class WorkspaceSessionGroups(INativeSettingsClient client, string projectId) : WorkspaceActionModel
{
    public NativeExplorationSummary[] Explorations { get; private set; } = [];
    public string? ExplorationError { get; private set; }
    private int explorationGeneration;
    public async Task<bool> LoadExplorationsAsync()
    {
        if (Closed) return false;
        var ticket = ++explorationGeneration;
        try
        {
            var reply = NativeExplorations.Decode(await client.InvokeAsync("native_conversation_explorations", new(), projectId), projectId);
            if (Closed || ticket != explorationGeneration) return false;
            Explorations = reply.Explorations; ExplorationError = null; return true;
        }
        catch (Exception error)
        {
            if (!Closed && ticket == explorationGeneration) ExplorationError = error.Message;
            return false;
        }
    }
    public ProjectFolder[] Folders { get; private set; } = [];
    public string Sort { get; set; } = "newest";
    public string Group { get; set; } = "folder";
    public bool Selecting { get; set; }
    public HashSet<string> Selected { get; } = [];
    public bool MoveUnconfirmed { get; private set; }
    public bool FolderSaveUnconfirmed { get; private set; }
    public bool FolderOrderUnconfirmed { get; private set; }
    // Folder dialogs provide their own localized uncertainty explanation.
    public string? FolderFailureDetail
    {
        get
        {
            const string prefix = "操作结果未能确认；不会自动重试。\n";
            return Error?.StartsWith(prefix, StringComparison.Ordinal) == true ? Error[prefix.Length..] : Error;
        }
    }
    private readonly HashSet<string> unconfirmedFolderDeletes = [];
    public bool CanDeleteFolder(string id) => !Closed && !Busy && Folders.Any(f => f.Id == id) && !unconfirmedFolderDeletes.Contains(id);
    public bool FolderDeleteUnconfirmed(string id) => unconfirmedFolderDeletes.Contains(id);
    private readonly HashSet<string> unconfirmedShelves = [];
    public bool CanShelve(string id) => !Closed && !Busy && !unconfirmedShelves.Contains(id);
    public Task<bool> ShelveAsync(string id)
    {
        if (!CanShelve(id)) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            try
            {
                var data = await client.InvokeAsync("native_project_session_shelve", new() { ["session_id"] = id, ["shelved"] = true }, projectId);
                var result = data?.Deserialize<SessionShelveResult>();
                if (result == null || result.ProjectId != projectId || result.SessionId != id || !result.Shelved)
                    throw new InvalidDataException("Shelve acknowledgement did not match the conversation");
                return true;
            }
            catch { unconfirmedShelves.Add(id); throw; }
        }, _ => { });
    }
    public Task<bool> MoveSessionAsync(string id, string? folderId) => MoveUnconfirmed ? Task.FromResult(false) : RunAsync(async () =>
    {
        try
        {
            var result = await client.InvokeAsync("native_project_session_move", new() { ["session_id"] = id, ["folder_id"] = folderId }, projectId);
            if (result is not JsonValue ack || !ack.TryGetValue<bool>(out var moved) || !moved) throw new InvalidDataException("Missing move acknowledgement");
            return true;
        }
        catch { MoveUnconfirmed = true; throw; }
    }, _ => { });
    public void ToggleSelection(IEnumerable<string> ids)
    {
        if (Busy || Closed) return;
        var members = ids.Distinct().ToArray();
        if (members.All(Selected.Contains)) Selected.ExceptWith(members); else Selected.UnionWith(members);
    }
    public string Draft { get; set; } = "";
    public string? RenamingId { get; set; }
    public bool CanSave => !Closed && !Busy && !FolderSaveUnconfirmed && !string.IsNullOrWhiteSpace(Draft);
    public void BeginFolderEdit(string? id)
    {
        RenamingId = id; Draft = id == null ? "" : Folders.FirstOrDefault(f => f.Id == id)?.Name ?? "";
        FolderSaveUnconfirmed = false; ClearFailure();
    }
    public Task<bool> LoadAsync() => RunAsync(
        async () => (await client.InvokeAsync("native_project_folders", new(), projectId))?.Deserialize<ProjectFolder[]>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing project folders"), value => Folders = value);

    public async Task<bool> SaveAsync()
    {
        if (string.IsNullOrWhiteSpace(Draft)) { Fail("请填写分组名称。"); return false; }
        if (!CanSave) return false;
        var args = new JsonObject { ["name"] = Draft.Trim() };
        var rename = RenamingId;
        if (rename != null) args["folder_id"] = rename;
        var saved = await RunAsync(async () =>
        {
            try
            {
                var result = await client.InvokeAsync(rename == null ? "native_project_folder_create" : "native_project_folder_rename", args, projectId);
                if (rename == null)
                {
                    var created = result?.Deserialize<ProjectFolder>(ConversationSnapshot.JsonOptions);
                    if (created == null || string.IsNullOrEmpty(created.Id) || created.Name != args["name"]!.GetValue<string>()) throw new InvalidDataException("Missing created folder acknowledgement");
                    return created;
                }
                else if (result is not JsonValue ack || !ack.TryGetValue<bool>(out var renamed) || !renamed) throw new InvalidDataException("Missing folder rename acknowledgement");
                return new ProjectFolder(rename, args["name"]!.GetValue<string>());
            }
            catch { FolderSaveUnconfirmed = true; throw; }
        }, saved => { Folders = Folders.Where(f => f.Id != saved.Id).Append(saved).ToArray(); Draft = ""; RenamingId = null; });
        if (saved) await LoadAsync();
        return saved;
    }
    public void BeginFolderDelete() { if (!Busy) ClearFailure(); }
    public Task<bool> DeleteFolderAsync(string id) => !CanDeleteFolder(id) ? Task.FromResult(false) : RunAsync(async () =>
    {
        try
        {
            var result = (await client.InvokeAsync("native_project_folder_delete", new() { ["folder_id"] = id }, projectId))?.Deserialize<NativeFolderDeleteResult>();
            if (result == null || result.ProjectId != projectId || result.FolderId != id || !result.Deleted) throw new InvalidDataException("Folder deletion acknowledgement did not match");
            return true;
        }
        catch { unconfirmedFolderDeletes.Add(id); throw; }
    }, _ => { Folders = Folders.Where(f => f.Id != id).ToArray(); collapsedFolders.Remove(id); });
    public bool CanReorderFolders => !Closed && !Busy && !FolderOrderUnconfirmed;
    public Task<bool> ReorderFoldersAsync(string[] ids)
    {
        if (!CanReorderFolders || ids.Length != Folders.Length || ids.Distinct().Count() != ids.Length
            || !ids.ToHashSet().SetEquals(Folders.Select(f => f.Id)) || ids.SequenceEqual(Folders.Select(f => f.Id))) return Task.FromResult(false);
        var order = ids.ToArray();
        return RunAsync(async () =>
        {
            try
            {
                var result = (await client.InvokeAsync("native_project_folders_reorder", new() { ["ids"] = JsonSerializer.SerializeToNode(order) }, projectId))?.Deserialize<NativeFolderOrderResult>();
                if (result == null || result.ProjectId != projectId || result.Ids == null || !result.Ids.SequenceEqual(order)) throw new InvalidDataException("Folder order acknowledgement did not match");
                return true;
            }
            catch { FolderOrderUnconfirmed = true; throw; }
        }, _ => { var folders = Folders.ToDictionary(f => f.Id); Folders = order.Select(id => folders[id]).ToArray(); });
    }
    public Task<bool> MoveAsync(string? folderId) => MoveUnconfirmed ? Task.FromResult(false) : RunAsync(async () =>
    {
        foreach (var id in Selected.ToArray())
        {
            if (Closed) return false;
            try
            {
                var result = await client.InvokeAsync("native_project_session_move", new() { ["session_id"] = id, ["folder_id"] = folderId }, projectId);
                if (result is not JsonValue ack || !ack.TryGetValue<bool>(out var moved) || !moved) throw new InvalidDataException("Missing move acknowledgement");
            }
            catch { MoveUnconfirmed = true; throw; }
            if (Closed) return false;
            // Do not replay earlier successes if a later move fails.
            Selected.Remove(id);
        }
        return true;
    }, moved => { if (moved) Selecting = false; });

    /// <summary>Rename the owned, unarchived conversation. The caller refreshes the session list;
    /// an ambiguous response is never retried automatically.</summary>
    public Task<bool> RenameAsync(string sessionId, string title) => RunAsync(async () =>
    {
        var trimmed = title.Trim();
        if (trimmed.Length == 0) throw new InvalidOperationException("请填写会话名称。");
        await client.InvokeAsync("native_conversation_rename", new() { ["session_id"] = sessionId, ["title"] = trimmed }, projectId);
        return true;
    }, _ => { });

    public Task<bool> PinAsync(string sessionId, bool pinned) => RunAsync(async () =>
    {
        await client.InvokeAsync("native_conversation_pin", new() { ["session_id"] = sessionId, ["pinned"] = pinned }, projectId);
        return true;
    }, _ => { });

    public Task<bool> DeleteAsync(string sessionId) => RunAsync(async () =>
    {
        await client.InvokeAsync("native_conversation_delete", new() { ["session_id"] = sessionId }, projectId);
        return true;
    }, _ => { });

    public NativeSessionNesting Nesting { get; } = new();
    private readonly HashSet<string> collapsedFolders = [];
    public bool FolderExpanded(string id) => !collapsedFolders.Contains(id);
    public void ToggleFolder(string id) { if (!collapsedFolders.Remove(id)) collapsedFolders.Add(id); }
    public NativeSessionTree Tree(IEnumerable<BrowserSession> sessions) => NativeSessionTree.Build(Order(sessions));
    private BrowserSession[] Order(IEnumerable<BrowserSession> sessions) => Sort == "name"
        ? sessions.OrderBy(s => s.Title.Trim().ToLowerInvariant(), StringComparer.Ordinal).ToArray()
        : sessions.ToArray(); // The host already orders by latest activity, not creation time.
    public SessionSection[] Sections(IEnumerable<BrowserSession> sessions, DateTimeOffset? now = null)
    {
        var ordered = Order(sessions);
        var result = new List<SessionSection>();
        var pinned = ordered.Where(s => s.Pinned == true).ToArray();
        if (pinned.Length > 0) result.Add(new("已置顶", null, pinned, "sidebar.pinned"));
        var unpinned = ordered.Where(s => s.Pinned != true).ToArray();
        void Dates(IEnumerable<BrowserSession> rows)
        {
            // Match bucket_sessions_by_date: a rolling 24 hours, with seconds and
            // legacy millisecond timestamps, rather than calendar-midnight buckets.
            var cutoff = (now ?? DateTimeOffset.UtcNow).ToUnixTimeMilliseconds() - 86_400_000m;
            bool Today(BrowserSession s) => s.Timestamp > 0
                && (s.Timestamp > 1_000_000_000_000 ? s.Timestamp : s.Timestamp * 1000m) >= cutoff;
            var list = rows.ToArray();
            var today = list.Where(Today).ToArray(); var earlier = list.Where(s => !Today(s)).ToArray();
            if (today.Length > 0) result.Add(new("今天", null, today, "sidebar.today"));
            if (earlier.Length > 0) result.Add(new("更早", null, earlier, "sidebar.earlier"));
        }
        if (Group == "date") Dates(unpinned);
        else if (Group == "folder")
        {
            foreach (var folder in Folders)
                result.Add(new(folder.Name, folder.Id, unpinned.Where(s => s.FolderId == folder.Id).ToArray()));
            // Keep an orphaned folder reference readable until the next folder refresh.
            Dates(unpinned.Where(s => !Folders.Any(f => f.Id == s.FolderId)));
        }
        else if (unpinned.Length > 0) result.Add(new("会话", null, unpinned));
        return result.ToArray();
    }
}

public sealed record ProjectCreationDraft(string Name, string Directory, string Description, string AgentContext, bool StandardLayout);

public sealed class WorkspaceProjectCreation(INativeProjectClient client, string? layoutContext = null, ProjectCreationDraft? draft = null) : WorkspaceActionModel
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
    public string Name { get; set; } = draft?.Name ?? "";
    public string Directory { get; set; } = draft?.Directory ?? "";
    public string Description { get; set; } = draft?.Description ?? "";
    public string AgentContext { get; set; } = draft?.AgentContext ?? "";
    public bool StandardLayout { get; set; } = draft?.StandardLayout ?? false;
    public ProjectCreationDraft CaptureDraft() => new(Name, Directory, Description, AgentContext, StandardLayout);
    public bool CanCreate => !Busy && !Closed
        && !string.IsNullOrWhiteSpace(Name) && !string.IsNullOrWhiteSpace(Directory);
    public void SetStandardLayout(bool enabled)
    {
        StandardLayout = enabled;
        var block = (string.IsNullOrEmpty(layoutContext) ? StandardContext : layoutContext).ReplaceLineEndings("\n");
        // WinUI TextBox may return CR/CRLF while the shared WebView catalog uses LF.
        // Compare canonical text so toggling a rendered block cannot duplicate it.
        var rest = AgentContext.ReplaceLineEndings("\n").Replace(block, "", StringComparison.Ordinal).Trim();
        AgentContext = enabled ? (rest.Length == 0 ? block : rest + "\n\n" + block) : rest;
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

public sealed partial class WorkspacePublicationModel(INativePublicationClient client, string projectId) : WorkspaceActionModel
{
    public string Title { get; set; } = "";
    public string Description { get; set; } = "";
    public string RevisionLabel { get; set; } = "";
    public NativePublicationWorkspace? Workspace { get; private set; }
    public bool CreationAvailable => Workspace is { Publications.Count: 0 };
    public bool CanCreate => CreationAvailable && !Busy && !Closed && !MutationUncertain
        && !string.IsNullOrWhiteSpace(Title) && !string.IsNullOrWhiteSpace(RevisionLabel);
    public async Task<bool> LoadAsync() {
        var ok = await RunAsync(() => client.SelectAsync(projectId, PublicationId, RevisionId), Apply);
        if (ok) { Reconciled = true; Notify(); } return ok;
    }
    public Task<bool> CreateAsync()
    {
        if (!CreationAvailable || Busy || Closed || MutationUncertain) return Task.FromResult(false);
        if (string.IsNullOrWhiteSpace(Title) || string.IsNullOrWhiteSpace(RevisionLabel))
        { Fail("请填写论文标题和版本标签。"); return Task.FromResult(false); }
        return CreateConfirmedAsync();
    }
}
