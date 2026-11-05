using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private static string Display(JsonNode? value) => value switch
    {
        null => "", JsonValue v when v.TryGetValue<bool>(out var enabled) => enabled ? "是" : "否",
        JsonValue v when v.TryGetValue<long>(out var number) => number.ToString("N0"),
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
}
