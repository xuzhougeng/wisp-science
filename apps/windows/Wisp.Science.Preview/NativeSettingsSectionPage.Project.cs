using System.Text.Json.Nodes;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private void ProjectSettings()
    {
        if (!projectScoped) { Results.Children.Add(Mute(L("请选择需要编辑的项目。", "Select a project to edit."))); return; }
        if (model.Values["get_project_settings"] is not JsonObject project) return;
        if (S(project, "id") != settingsProjectId) { Results.Children.Add(Warn(L("返回的项目与当前作用域不符，请重新打开项目设置。", "The returned project does not match this scope. Reopen project settings."))); return; }
        var card = Card(S(project, "name"));
        card.Children.Add(Mute(S(project, "description")));
        card.Children.Add(Mute(L("Agent Context 保存到该项目的 .wisp/WISP.md。", "Agent Context is saved to this project's .wisp/WISP.md.")));
        if (B(project, "folder_sync")) card.Children.Add(Mute(L("此项目使用同步文件夹。", "This project uses a synced folder.")));
        card.Children.Add(Button(L("编辑项目设置", "Edit project settings"), () =>
        {
            Editor(T("proj_settings.title"), new() { ["name"] = S(project, "name"), ["description"] = S(project, "description"), ["agent_context"] = S(project, "agent_context") }, "update_project", null,
                d => { Text(d, "name", T("proj_settings.name")); Text(d, "description", T("proj_settings.description"), true); Text(d, "agent_context", T("proj_settings.agent_context"), true); }, new() { ["id"] = settingsProjectId });
            return Task.CompletedTask;
        }));
        Preference(T("proj_settings.retention"), "get_project_run_retention", "set_project_run_retention", null, d =>
        {
            Form.Children.Add(Mute(T("proj_settings.retention_hint")));
            Integer(d, "run_retention_days", T("proj_settings.retention_succeeded"), nullable: true);
            Integer(d, "failed_run_retention_days", T("proj_settings.retention_failed"), nullable: true);
            Integer(d, "orphan_file_retention_days", T("proj_settings.retention_orphan"), nullable: true);
        });
    }
}
