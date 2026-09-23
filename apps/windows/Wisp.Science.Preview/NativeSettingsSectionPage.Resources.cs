using System.Diagnostics;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private static string Display(JsonNode? value) => value switch
    {
        null => "", JsonValue v when v.TryGetValue<bool>(out var enabled) => enabled ? "是" : "否",
        JsonArray a => $"{a.Count} 项", _ => value.ToString()
    };
    private void Summary(StackPanel parent, JsonNode? value)
    {
        var labels = new Dictionary<string, string> { ["id"] = "标识", ["label"] = "名称", ["name"] = "名称", ["status"] = "状态", ["message"] = "说明",
            ["kind"] = "类型", ["last_probe_status"] = "探测状态", ["last_probe_error"] = "探测错误", ["last_probe_at"] = "上次探测",
            ["capabilities_json"] = "能力详情", ["config_json"] = "环境配置", ["created_at"] = "创建时间", ["updated_at"] = "更新时间",
            ["context_id"] = "执行环境", ["project_id"] = "项目", ["remote_files"] = "远程文件", ["local_files"] = "本地文件", ["blockers"] = "阻止清理的项目", ["warnings"] = "提示" };
        if (value is JsonArray array) { foreach (var row in array) Summary(parent, row); return; }
        if (value is not JsonObject obj) { parent.Children.Add(Mute(Display(value))); return; }
        foreach (var (key, item) in obj)
        {
            if (item == null) continue;
            var label = labels.GetValueOrDefault(key, key);
            if (item is JsonObject or JsonArray)
            {
                var nested = new StackPanel { Spacing = 5 }; Summary(nested, item);
                parent.Children.Add(Disclosure(label, nested));
            }
            else parent.Children.Add(Mute(label + "：" + Display(item)));
        }
    }
    private async Task ReadDetail(StackPanel target, string command, JsonObject args)
    {
        JsonNode? value = null;
        if (await model.InvokeAsync(command, args, v => value = v)) { target.Children.Clear(); Summary(target, value); }
    }
    private void Environments()
    {
        var contexts = Rows(model.Values["list_execution_contexts"]).ToArray();
        var defaults = Card("默认远程环境");
        var options = new[] { (Id: "", Label: "无（始终可使用本机）") }.Concat(contexts.Where(c => S(c, "kind") != "local").Select(c => (Id: S(c, "id"), Label: S(c, "label")))).ToArray();
        Choice(defaults, "默认环境", model.Values["get_default_execution_context"]?.GetValue<string>() ?? "", options,
            id => _ = Run("set_default_execution_context", new() { ["contextId"] = id.Length == 0 ? null : id }));
        Action(defaults, "导入本机 WSL 环境", "import_wsl_contexts", new());
        foreach (var context in contexts)
        {
            var id = S(context, "id"); var card = Card(S(context, "label") is { Length: > 0 } label ? label : id);
            card.Children.Add(Mute(S(context, "kind"))); var detail = new StackPanel { Spacing = 5 };
            card.Children.Add(Button("探测环境", () => ReadDetail(detail, "probe_execution_context", new() { ["contextId"] = id })));
            card.Children.Add(Button("编辑解释器", () =>
            {
                JsonObject config;
                try { config = JsonNode.Parse(S(context, "config_json")) as JsonObject ?? new(); }
                catch (System.Text.Json.JsonException ex) { model.Fail("执行环境配置无法解析：" + ex.Message); return Task.CompletedTask; }
                Editor("解释器路径", new() { ["python_executable"] = config["python_executable"]?.DeepClone(), ["rscript_executable"] = config["rscript_executable"]?.DeepClone() },
                    "update_execution_context_interpreters", null, d => { Text(d, "python_executable", "Python"); Text(d, "rscript_executable", "Rscript"); }, new() { ["contextId"] = id }); return Task.CompletedTask;
            }));
            if (projectScoped)
            {
                card.Children.Add(Button("编辑存储路径", async () =>
                {
                    JsonObject? prefs = null;
                    if (await model.InvokeAsync("get_context_storage_prefs", new() { ["contextId"] = id }, v => prefs = v as JsonObject) && prefs != null)
                        Editor("执行环境存储", prefs, "set_context_storage_prefs", null, d =>
                        { Text(d, "remote_data_root", "远程数据目录"); Text(d, "remote_workdir_root", "远程运行目录"); PathField(d, "local_results_dir", "本地结果目录", directory: true); }, new() { ["contextId"] = id });
                }));
                card.Children.Add(Button("清理报告", () => ReadDetail(detail, "context_disposal_report", new() { ["contextId"] = id })));
            }
            card.Children.Add(detail);
        }
        var hosts = Card("SSH 主机");
        hosts.Children.Add(Button("添加主机", () => { EditHost(new() { ["alias"] = "", ["port"] = 22, ["auth_method"] = "key" }); return Task.CompletedTask; }));
        Action(hosts, "导入 SSH 配置", "import_ssh_config_hosts", new());
        foreach (var host in Rows(model.Values["list_ssh_hosts"]))
        {
            var card = Card(S(host, "alias")); card.Children.Add(Mute(S(host, "host_name")));
            card.Children.Add(Button("编辑主机", () => { EditHost(host); return Task.CompletedTask; }));
            Action(card, "测试连接", "test_ssh_connection", new() { ["host"] = host.DeepClone() });
            Action(card, "移除主机", "remove_ssh_host", new() { ["alias"] = S(host, "alias") }, true);
        }
        foreach (var edge in Rows(model.Values["list_ssh_trust_edges"]))
        {
            var card = Card(S(edge, "source_context_id") + " → " + S(edge, "destination_context_id"));
            Action(card, "撤销服务器信任", "revoke_ssh_trust_edge", new() { ["sourceContextId"] = S(edge, "source_context_id"), ["destinationContextId"] = S(edge, "destination_context_id") }, true);
        }
    }
    private void EditHost(JsonObject host) => Editor("SSH 主机", host, "add_ssh_host", "host", d =>
    {
        Text(d, "alias", "别名"); Text(d, "host_name", "主机地址"); Text(d, "user", "用户名"); Integer(d, "port", "端口", 1, 65535);
        Choice(Form, "认证方式", S(d, "auth_method"), [("key", "密钥 / SSH 配置"), ("password", "密码")], v => d["auth_method"] = v);
        PathField(d, "identity_file", "私钥文件路径"); Secure(d, "password", "密码（留空保留已有密码）");
        Form.Children.Add(Mute("私钥只保存文件路径；密码使用现有凭据存储。")); Text(d, "notes", "备注", true);
    });
    private static string Bytes(JsonNode? value)
    {
        if (value == null || !long.TryParse(value.ToString(), out var size)) return "0 B";
        return size >= 1024L * 1024 * 1024 ? $"{size / (1024d * 1024 * 1024):0.0} GB" : size >= 1024 * 1024 ? $"{size / (1024d * 1024):0.0} MB" : size >= 1024 ? $"{size / 1024d:0.0} KB" : $"{size} B";
    }
    private void RevealPath(StackPanel card, string path)
    {
        card.Children.Add(Button("在资源管理器中显示", () =>
        {
            try
            {
                if (!Directory.Exists(path)) throw new DirectoryNotFoundException("目录不存在或当前无法访问。");
                var start = new ProcessStartInfo("explorer.exe") { UseShellExecute = false }; start.ArgumentList.Add(Path.GetFullPath(path)); Process.Start(start);
            }
            catch (Exception ex) { model.Fail(ex.Message); }
            return Task.CompletedTask;
        }));
    }
    private void Storage()
    {
        var usage = model.Values["get_storage_usage"];
        if (usage != null)
        {
            var card = Card("本地存储 · " + Bytes(usage["total_bytes"])); card.Children.Add(Mute(S(usage, "data_dir"))); RevealPath(card, S(usage, "data_dir"));
            foreach (var row in Rows(usage["entries"])) card.Children.Add(Mute(S(row, "key") + " · " + Bytes(row["bytes"])));
            foreach (var row in Rows(usage["projects"])) { var project = Card(S(row, "name") + " · " + Bytes(row["bytes"])); RevealPath(project, S(row, "path")); }
        }
        if (projectScoped) Preference("项目保留策略", "get_project_run_retention", "set_project_run_retention", null, d =>
        {
            Form.Children.Add(Mute("留空使用默认值。修改策略不会立即删除文件。"));
            Integer(d, "run_retention_days", "成功运行保留天数", nullable: true);
            Integer(d, "failed_run_retention_days", "失败运行保留天数", nullable: true);
            Integer(d, "orphan_file_retention_days", "孤立文件保留天数", nullable: true);
        });
    }
    private void Usage()
    {
        var usage = model.Values["get_token_usage"];
        foreach (var workspace in Rows(usage?["workspaces"]))
        {
            var card = Card(S(workspace, "name"));
            card.Children.Add(Mute($"输入 {Display(workspace["input"])} · 输出 {Display(workspace["output"])} · 缓存 {Display(workspace["cached"])}"));
            var list = new StackPanel { Spacing = 6 }; var offset = 0; var total = 0; Button? more = null;
            async Task Load()
            {
                JsonNode? result = null;
                if (!await model.InvokeAsync("get_session_token_usage", new() { ["projectId"] = S(workspace, "project_id"), ["offset"] = offset, ["limit"] = 30 }, v => result = v)) return;
                var rows = Rows(result?["items"]).ToArray();
                foreach (var row in rows) list.Children.Add(Mute($"{S(row, "title")} · 输入 {Display(row["input"])} / 输出 {Display(row["output"])}"));
                offset += rows.Length; total = result?["total"]?.GetValue<int>() ?? offset;
                if (more != null) more.Visibility = offset < total && rows.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            }
            more = Button("查看 / 加载更多会话", Load); card.Children.Add(more); card.Children.Add(list);
        }
        foreach (var (title, key, name, metric) in new[] { ("模型用量", "models", "model", "tokens"), ("工具调用", "tools", "name", "calls"), ("每日 Token", "days", "date", "tokens") })
        {
            var card = Card(title); foreach (var row in Rows(usage?[key])) card.Children.Add(Mute(S(row, name) + " · " + Display(row[metric])));
        }
    }
}
