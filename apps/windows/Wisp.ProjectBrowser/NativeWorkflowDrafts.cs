using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// Draft construction keeps persisted identities and nullable capability inheritance explicit.
public static class NativeWorkflowDrafts
{
    public static JsonObject Workflow() => new()
    {
        ["id"] = Guid.NewGuid().ToString(), ["name"] = "", ["description"] = "", ["builtin"] = false,
        ["proposal"] = new JsonObject { ["goal"] = "", ["context"] = "", ["approval_policy"] = "review_all", ["tasks"] = new JsonArray() }
    };
    public static JsonObject TaskNode(IEnumerable<JsonObject> existing)
    {
        var ids = existing.Select(n => n["id"]?.GetValue<string>()).ToHashSet();
        var index = 1; while (ids.Contains("task" + index)) index++;
        return new() { ["id"] = "task" + index, ["instruction"] = "", ["depends_on"] = new JsonArray(),
            ["capabilities"] = new JsonArray(), ["task_kind"] = "agent", ["isolated"] = false };
    }
    public static JsonObject QuickAction(int order) => new()
    {
        ["id"] = Guid.NewGuid().ToString(), ["name"] = "", ["description"] = "",
        ["builtin"] = false, ["context"] = "selection", ["enabled"] = true,
        ["sort_order"] = order, ["icon"] = "bolt", ["workflow_template_id"] = ""
    };
    public static JsonObject Specialist() => new()
    {
        ["id"] = Guid.NewGuid().ToString(), ["name"] = "", ["description"] = "",
        ["builtin"] = false, ["instructions"] = "", ["model_id"] = "",
        ["skills"] = null, ["connectors"] = null
    };
    public static JsonObject Copy(JsonObject source)
    {
        var copy = (JsonObject)source.DeepClone();
        copy["id"] = Guid.NewGuid().ToString();
        copy["name"] = (source["name"]?.GetValue<string>() ?? "") + " 副本";
        copy["builtin"] = false;
        // Reviewer backends belong only to the built-in reviewer identity.
        if (source["id"]?.GetValue<string>() == "reviewer") copy.Remove("review_backend");
        return copy;
    }
    public static void SetWhitelist(JsonObject draft, string key, bool inherit, string text)
        => draft[key] = inherit ? null : new JsonArray(text.Split('\n').Select(s => s.Trim())
            .Where(s => s.Length > 0).Distinct(StringComparer.Ordinal).Select(s => (JsonNode?)JsonValue.Create(s)).ToArray());
}
