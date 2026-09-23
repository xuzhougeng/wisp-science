using System.Text.Json.Nodes;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private void ProjectSettings()
    {
        if (!projectScoped) { Results.Children.Add(Mute("请选择需要编辑的项目。")); return; }
        if (model.Values["get_project_settings"] is not JsonObject project) return;
        if (S(project, "id") != settingsProjectId) { Results.Children.Add(Warn("返回的项目与当前作用域不符，请重新打开项目设置。")); return; }
        var card = Card(S(project, "name"));
        card.Children.Add(Mute(S(project, "description")));
        card.Children.Add(Mute("Agent Context 保存到该项目的 .wisp/WISP.md。"));
        if (B(project, "folder_sync")) card.Children.Add(Mute("此项目使用同步文件夹。"));
        card.Children.Add(Button("编辑项目设置", () =>
        {
            Editor("项目设置", new() { ["name"] = S(project, "name"), ["description"] = S(project, "description"), ["agent_context"] = S(project, "agent_context") }, "update_project", null,
                d => { Text(d, "name", "项目名称"); Text(d, "description", "项目说明", true); Text(d, "agent_context", "Agent Context", true); }, new() { ["id"] = settingsProjectId });
            return Task.CompletedTask;
        }));
    }
}
