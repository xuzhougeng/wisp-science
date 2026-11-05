using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Memory browsing owns its project scope independently of the active conversation.
public sealed class NativeMemoryModel(INativeSettingsClient client, string? initialProject, NativeLocalization? localization = null) : WorkspaceActionModel
{
    public IReadOnlyList<JsonObject> Projects { get; private set; } = [];
    public JsonObject? View { get; private set; }
    public string ProjectId => S(View, "project_id");
    public string TodayFile => S(View, "today_file");
    public bool Enabled => View?["enabled"]?.GetValue<bool>() == true;
    public IReadOnlyList<JsonObject> Files => Rows(View?["files"]);
    public IReadOnlyList<JsonObject> GlobalMemories => Rows(View?["global_memories"]);
    public string? NoteName { get; private set; }
    /// Null means closed; empty means a new global habit.
    public string? GlobalId { get; private set; }
    public string Content { get; private set; } = "";
    private string original = "";
    public string? Feedback { get; private set; }
    public bool HasDraft => NoteName != null || GlobalId != null;
    public bool HasChanges => HasDraft && Content != original;
    public bool CanSave => !Busy && !Closed && ProjectId.Length > 0 && (NoteName != null || GlobalId != null && Content.Trim().Length > 0);
    private static string S(JsonNode? row, string key) => row?[key]?.GetValue<string>() ?? "";
    private static IReadOnlyList<JsonObject> Rows(JsonNode? value) => (value as JsonArray)?.OfType<JsonObject>().ToArray() ?? [];
    private string L(string zh, string en) => localization?.Choose(zh, en) ?? zh;
    protected override string UnconfirmedFailure(Exception exception) => L("操作结果未能确认；不会自动重试。",
        "The operation could not be confirmed; it will not be retried automatically.") + "\n" + exception.Message;
    private JsonArray Array(JsonNode? value) => value is JsonArray rows ? (JsonArray)rows.DeepClone() : throw new InvalidDataException(L("记忆列表响应无效。", "Invalid memory list response."));
    private JsonObject MemoryView(JsonNode? value, string project)
    {
        if (value is not JsonObject view || S(view, "project_id") != project || view["files"] is not JsonArray || view["global_memories"] is not JsonArray)
            throw new InvalidDataException(L("记忆响应与所选项目不一致。", "The memory response does not match the selected project."));
        return (JsonObject)view.DeepClone();
    }
    private static JsonObject Scope(string project) => new() { ["projectId"] = project };
    public Task<bool> LoadAsync()
    {
        if (HasDraft) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            var projects = Rows(Array(await client.InvokeAsync("list_projects", new())));
            var preferred = ProjectId.Length > 0 ? ProjectId : initialProject;
            var id = projects.Count == 0 && preferred == null ? null
                : await NativeSettingsWorkspace.ResolveAsync(client, preferred);
            var view = id == null ? null : MemoryView(await client.InvokeAsync("get_memory_view", Scope(id), id), id);
            return (projects, view);
        }, result => { Projects = result.projects; View = result.view; Feedback = null; });
    }
    public Task<bool> SelectProjectAsync(string id)
    {
        if (HasDraft || id.Length == 0 || !Projects.Any(p => S(p, "id") == id)) return Task.FromResult(false);
        if (id == ProjectId) return Task.FromResult(!Busy && !Closed);
        return RunAsync(async () => MemoryView(await client.InvokeAsync("get_memory_view", Scope(id), id), id), view => { View = view; Feedback = null; });
    }
    public Task<bool> OpenNoteAsync(string name)
    {
        if (HasDraft || ProjectId.Length == 0 || string.IsNullOrWhiteSpace(name)) return Task.FromResult(false);
        var project = ProjectId;
        return RunAsync(async () =>
        {
            var args = Scope(project); args["name"] = name;
            return (await client.InvokeAsync("read_memory_file", args, project))?.GetValue<string>()
                ?? throw new InvalidDataException(L("无法读取记忆内容。", "Could not read memory content."));
        }, content => { NoteName = name; GlobalId = null; Content = original = content; Feedback = null; });
    }
    public bool EditGlobal(string id = "")
    {
        if (Busy || Closed || HasDraft || View == null) return false;
        var row = id.Length == 0 ? null : GlobalMemories.FirstOrDefault(r => S(r, "id") == id);
        if (id.Length > 0 && row == null) return false;
        NoteName = null; GlobalId = id; Content = original = S(row, "content"); Feedback = null; ClearFailure(); Notify(); return true;
    }
    public void EditContent(string content)
    {
        if (Busy || Closed || !HasDraft) return;
        Content = content; Feedback = null; Notify();
    }
    public void Discard()
    {
        if (Busy) return;
        NoteName = GlobalId = null; Content = original = ""; Feedback = null; ClearFailure(); Notify();
    }
    public async Task<bool> SaveAsync()
    {
        if (!CanSave) return false;
        var project = ProjectId; var note = NoteName; var globalId = GlobalId;
        var content = note == null ? Content.Trim() : Content;
        return await RunAsync(async () =>
        {
            if (note != null)
            {
                var args = Scope(project); args["name"] = note; args["content"] = content;
                return (JsonNode)Array(await client.InvokeAsync("write_memory_file", args, project));
            }
            var globalArgs = new JsonObject { ["content"] = content };
            if (globalId!.Length > 0) globalArgs["id"] = globalId;
            var value = await client.InvokeAsync(globalId.Length == 0 ? "create_global_memory" : "update_global_memory", globalArgs, project);
            if (globalId.Length > 0)
            {
                // The update command acknowledges with void; preserve the loaded
                // row's identity/provenance rather than expecting a create response.
                var updated = (JsonObject)(GlobalMemories.FirstOrDefault(row => S(row, "id") == globalId)
                    ?? throw new InvalidDataException(L("全局习惯已不在当前列表中。", "This global habit is no longer in the current list."))).DeepClone();
                updated["content"] = content; return updated;
            }
            if (value is not JsonObject row || S(row, "id").Length == 0 || row["content"] is not JsonValue)
                throw new InvalidDataException(L("全局习惯保存响应无效。", "Invalid acknowledgement for the saved global habit."));
            return row.DeepClone();
        }, result =>
        {
            if (note != null) { View!["files"] = result; Content = original = content; }
            else
            {
                var savedId = S(result, "id");
                var rows = GlobalMemories.Select(r => (JsonNode?)r.DeepClone()).ToList();
                var index = rows.FindIndex(r => S(r, "id") == savedId);
                if (index >= 0) rows[index] = result; else rows.Insert(0, result);
                View!["global_memories"] = new JsonArray(rows.ToArray());
                NoteName = GlobalId = null; Content = original = "";
            }
            Feedback = localization?.Text(note != null ? "memory.save" : globalId!.Length == 0 ? "memory.global_created" : "memory.global_updated") ?? "已保存。";
        });
    }
    public Task<bool> SetEnabledAsync(bool enabled)
    {
        if (HasDraft || ProjectId.Length == 0) return Task.FromResult(false);
        var project = ProjectId; var args = Scope(project); args["enabled"] = enabled;
        return RunAsync(async () => MemoryView(await client.InvokeAsync("set_memory_enabled", args, project), project), view => { View = view; Feedback = null; });
    }
    public Task<bool> DeleteNoteAsync()
    {
        if (NoteName == null || ProjectId.Length == 0) return Task.FromResult(false);
        var project = ProjectId; var args = Scope(project); args["name"] = NoteName;
        return RunAsync(async () => Array(await client.InvokeAsync("delete_memory_file", args, project)), files =>
        { View!["files"] = files; NoteName = GlobalId = null; Content = original = ""; Feedback = localization?.Choose("记忆文件已删除。", "Memory file deleted.") ?? "记忆文件已删除。"; });
    }
    public Task<bool> ClearAsync()
    {
        if (HasDraft || ProjectId.Length == 0) return Task.FromResult(false);
        var project = ProjectId;
        return RunAsync(async () => Array(await client.InvokeAsync("clear_memory", Scope(project), project)), files => { View!["files"] = files; Feedback = localization?.Choose("项目记忆已清空。", "Project memory cleared.") ?? "项目记忆已清空。"; });
    }
    public Task<bool> DeleteGlobalAsync(string id)
    {
        if (HasDraft || View == null || !GlobalMemories.Any(row => S(row, "id") == id)) return Task.FromResult(false);
        var project = ProjectId;
        return RunAsync(() => client.InvokeAsync("delete_global_memory", new() { ["id"] = id }, project), _ =>
        { View!["global_memories"] = new JsonArray(GlobalMemories.Where(row => S(row, "id") != id).Select(row => (JsonNode?)row.DeepClone()).ToArray()); Feedback = localization?.Text("memory.global_deleted") ?? "全局习惯已删除。"; });
    }
    public override void Dispose() { base.Dispose(); Projects = []; View = null; NoteName = GlobalId = null; Content = original = ""; }
}
