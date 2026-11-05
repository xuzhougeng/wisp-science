using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;
using static Wisp.ProjectBrowser.NativeSkillsModel;

namespace Wisp.ProjectBrowser;

public sealed record NativePluginLaunch(string ProjectId, string SessionId, string Prompt, string[] SkillNames);

/// Project-scoped bindings over globally installed plugin versions.
public sealed class NativePluginsModel(INativeSettingsClient client, string? initialProject, NativeLocalization? localization = null) : WorkspaceActionModel
{
    public IReadOnlyList<JsonObject> Plugins { get; private set; } = [];
    public string? ProjectId { get; private set; } = initialProject;
    public bool Loaded { get; private set; }
    public string Query { get; set; } = "";
    public bool Installing { get; private set; }
    public bool Remote { get; private set; }
    public string LocalPath { get; private set; } = "";
    public string SourceUrl { get; private set; } = "";
    public string Checksum { get; private set; } = "";
    public JsonObject? Removal { get; private set; }
    public NativePluginLaunch? Launch { get; private set; }
    public string? Notice { get; private set; }
    public bool ModalOpen => Installing || Removal != null;
    private static JsonObject[] Rows(JsonNode? value) => value is JsonArray array && array.All(row => row is JsonObject)
        ? array.OfType<JsonObject>().Select(row => (JsonObject)row.DeepClone()).ToArray() : throw new InvalidDataException("插件列表响应无效。");
    public static string Identity(JsonObject row) => S(row, "id") + "\n" + S(row, "version");
    public static bool Matches(JsonObject row, string query) => new[] { "display_name", "id", "description" }.Any(field => S(row, field).Contains(query.Trim(), StringComparison.OrdinalIgnoreCase));
    public JsonObject[] Visible => Plugins.Where(row => Matches(row, Query)).ToArray();
    public int EnabledCount => Plugins.Count(row => B(row, "enabled"));
    public static bool ValidChecksum(string value) => value.Trim().Length == 64 && value.Trim().All(Uri.IsHexDigit);
    public bool CanInstall => Installing && !Busy && !Closed && (Remote
        ? SourceUrl.Trim().Length > 0 && ValidChecksum(Checksum)
        : LocalPath.Length > 0 && (Checksum.Trim().Length == 0 || ValidChecksum(Checksum)));
    public static string RuntimeLabel(JsonObject row, NativeLocalization? locale = null) => locale?.Text(S(row, "runtime_status") switch
        { "ready" => "plugins.runtime_ready", "unavailable" => "plugins.runtime_unavailable", _ => "plugins.runtime_not_applicable" })
        ?? (S(row, "runtime_status") switch { "ready" => "运行环境就绪", "unavailable" => "运行环境不可用", _ => "无需运行环境" });
    public static string StartPrompt(JsonObject row) => Strings(row["skill_names"]).Length > 0
        ? $"现在使用已启用的 {S(row, "display_name")} 插件。它的技能说明已附加到本条消息，请遵循说明并立即调用插件中启动或打开交互式工作台的工具。不要按显示名称搜索插件，也不要只描述能力。启动失败时，请报告准确错误。"
        : $"现在使用已启用的 {S(row, "display_name")} 插件。立即发现并调用它的启动或打开工作台 MCP 工具。不要只描述它的能力。启动失败时，请报告准确错误。";
    public Task<bool> LoadAsync()
    {
        if (ModalOpen) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            var project = await NativeSettingsWorkspace.ResolveAsync(client, ProjectId);
            var plugins = Rows(await client.InvokeAsync("list_plugins", new(), project));
            return (project, plugins);
        }, result => { ProjectId = result.project; Plugins = result.plugins; Loaded = true; });
    }
    public bool OpenInstall()
    {
        if (Busy || Closed || ModalOpen || ProjectId == null) return false;
        Installing = true; Remote = false; LocalPath = SourceUrl = Checksum = ""; Notice = null; ClearFailure(); Notify(); return true;
    }
    public void SetRemote(bool remote) { if (!Busy && !Closed && Installing) { Remote = remote; ClearFailure(); Notify(); } }
    public void EditInstall(string? url = null, string? checksum = null, string? path = null)
    {
        if (Busy || Closed || !Installing) return;
        if (url != null) SourceUrl = url; if (checksum != null) Checksum = checksum; if (path != null) LocalPath = path;
        Notify();
    }
    public bool CloseModal()
    {
        if (!ModalOpen || Busy || Closed) return false;
        Installing = false; Removal = null; LocalPath = SourceUrl = Checksum = ""; ClearFailure(); Notify(); return true;
    }
    public Task<bool> InstallAsync()
    {
        if (!CanInstall) return Task.FromResult(false);
        var remote = Remote; var args = new JsonObject { [remote ? "sourceUrl" : "srcPath"] = remote ? SourceUrl.Trim() : LocalPath,
            ["expectedSha256"] = Checksum.Trim().Length == 0 ? null : Checksum.Trim() };
        return RunAsync(async () =>
        {
            var row = await client.InvokeAsync(remote ? "install_plugin_url" : "install_plugin", args, ProjectId) as JsonObject;
            if (row == null || S(row, "id").Length == 0 || S(row, "version").Length == 0) throw new InvalidDataException("安装响应未包含插件身份，请刷新核对结果。");
            return row;
        }, row =>
        {
            Plugins = Plugins.Where(item => Identity(item) != Identity(row)).Append((JsonObject)row.DeepClone()).OrderBy(item => S(item, "display_name"), StringComparer.OrdinalIgnoreCase).ToArray();
            Installing = false; LocalPath = SourceUrl = Checksum = ""; Notice = localization?.Text("plugins.installed") ?? "插件已安装或更新。启用后才会启动它的 MCP 服务。";
        });
    }
    private bool Owns(JsonObject row) => Plugins.Contains(row) && S(row, "id").Length > 0 && S(row, "version").Length > 0;
    private static JsonObject Arguments(JsonObject row) => new() { ["pluginId"] = S(row, "id"), ["version"] = S(row, "version") };
    private async Task Enable(JsonObject row, bool enabled)
    {
        var args = Arguments(row); args["enabled"] = enabled; await client.InvokeAsync("set_plugin_enabled", args, ProjectId);
        if (Closed) return;
        // The backend binds one version of a plugin to a project.
        if (enabled) foreach (var item in Plugins.Where(item => S(item, "id") == S(row, "id"))) item["enabled"] = false;
        row["enabled"] = enabled;
    }
    public Task<bool> SetEnabledAsync(JsonObject row, bool enabled)
    {
        if (ModalOpen || ProjectId == null || !Owns(row)) return Task.FromResult(false);
        return RunAsync(async () => { await Enable(row, enabled); return true; }, _ => Notice = null);
    }
    public bool RequestRemoval(JsonObject row)
    {
        if (Busy || Closed || ModalOpen || !Owns(row)) return false;
        Removal = row; ClearFailure(); Notify(); return true;
    }
    public Task<bool> RemoveAsync()
    {
        if (Removal is not { } row || !Owns(row)) return Task.FromResult(false);
        return RunAsync(() => client.InvokeAsync("remove_plugin", Arguments(row), ProjectId), _ =>
        { Plugins = Plugins.Where(item => Identity(item) != Identity(row)).ToArray(); Removal = null; Notice = localization?.Choose("插件已移除。", "Plugin removed.") ?? "插件已移除。"; });
    }
    public Task<bool> UseAsync(JsonObject row)
    {
        if (ModalOpen || ProjectId == null || !Owns(row) || S(row, "runtime_status") == "unavailable") return Task.FromResult(false);
        var project = ProjectId; var skills = Strings(row["skill_names"]).Where(name => !string.IsNullOrWhiteSpace(name)).Distinct(StringComparer.Ordinal).ToArray();
        if (skills.Length > 64) { Fail("插件技能数量超过会话引用上限。"); return Task.FromResult(false); }
        return RunAsync(async () =>
        {
            if (!B(row, "enabled")) await Enable(row, true);
            if (Closed) return null;
            var session = await new NativeConversationClient(client).CreateAsync(project);
            if (string.IsNullOrWhiteSpace(session)) throw new InvalidDataException("未能确认新会话，请刷新查看。");
            return new NativePluginLaunch(project, session, StartPrompt(row), skills);
        }, launch => Launch = launch);
    }
}
