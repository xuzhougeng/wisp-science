using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public static class NativeQuickActions
{
    // Match WebView's exact built-in label rule; renamed actions remain user text.
    public static string Label(JsonObject action, NativeLocalization localization) =>
        action["id"]?.GetValue<string>() == "literature_research" && action["name"]?.GetValue<string>() == "Research literature"
            ? localization.Text("selection.research_literature") : action["name"]?.GetValue<string>() ?? "";
    public static JsonObject Create(IEnumerable<JsonObject> actions, IEnumerable<JsonObject> workflows)
    {
        var maximum = actions.Select(action => long.TryParse(action["sort_order"]?.ToString(), out var order) ? order : 0).DefaultIfEmpty(0).Max();
        var draft = NativeWorkflowDrafts.QuickAction(0);
        draft["id"] = ""; draft["icon"] = "sparkles";
        draft["sort_order"] = maximum > long.MaxValue - 10 ? long.MaxValue : maximum + 10;
        draft["workflow_template_id"] = workflows.FirstOrDefault()?["id"]?.GetValue<string>() ?? "";
        return draft;
    }
    public static string WorkflowName(JsonObject action, IEnumerable<JsonObject> workflows)
    {
        var id = action["workflow_template_id"]?.GetValue<string>() ?? "";
        return workflows.FirstOrDefault(workflow => workflow["id"]?.GetValue<string>() == id)?["name"]?.GetValue<string>() ?? id;
    }
    public static string? SaveBlockReason(JsonObject action) =>
        string.IsNullOrWhiteSpace(action["name"]?.GetValue<string>()) ? "请填写动作名称。"
        : string.IsNullOrWhiteSpace(action["workflow_template_id"]?.GetValue<string>()) ? "请选择工作流。" : null;
}
