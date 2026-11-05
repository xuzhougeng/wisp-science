using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// Catalog IDs stay opaque; editing one option preserves unknown selections.
public static class NativeWorkflowOptions
{
    // These document reviewers cannot execute delegated workflow tasks.
    public static bool CanAssignSpecialist(string id) => id is not ("archivist" or "recap");

    public static string ModelOptionLabel(JsonObject option)
    {
        var label = NativeWorkflowStudioModel.Text(option, "label");
        if (label.Length == 0) label = NativeWorkflowStudioModel.Text(option, "id");
        return option["external"]?.GetValue<bool>() == true ? label + " · external" : label;
    }
    public static string[] CapabilityIds(JsonObject task) => (task["capabilities"] as JsonArray ?? [])
        .Select(node => node?.GetValue<string>() ?? "").Where(id => id.Length > 0).ToArray();
    public static string CapabilitySummary(JsonObject task, IReadOnlyList<JsonObject> catalog)
    {
        var ids = CapabilityIds(task);
        var names = ids.Take(3).Select(id => NativeWorkflowStudioModel.Text(
            catalog.FirstOrDefault(row => NativeWorkflowStudioModel.Text(row, "id") == id), "display_name") is { Length: > 0 } label ? label : id);
        return string.Join(" · ", names) + (ids.Length > 3 ? $" +{ids.Length - 3}" : "");
    }
    public static void SetCapability(JsonObject task, string id, bool enabled)
    {
        var ids = CapabilityIds(task).ToList();
        if (enabled && !ids.Contains(id, StringComparer.Ordinal)) ids.Add(id);
        if (!enabled) ids.RemoveAll(value => value == id);
        task["capabilities"] = new JsonArray(ids.Select(value => (JsonNode?)JsonValue.Create(value)).ToArray());
    }
    public static string ExecutorKey(JsonNode? executor)
    {
        var kind = NativeWorkflowStudioModel.Text(executor, "kind");
        var profile = NativeWorkflowStudioModel.Text(executor, "profile_id");
        return profile.Length == 0 ? kind : kind + ":" + profile;
    }
    public static JsonObject? Executor(string key)
    {
        if (string.IsNullOrWhiteSpace(key)) return null;
        var parts = key.Split(':', 2);
        return new JsonObject { ["kind"] = parts[0], ["profile_id"] = parts.Length == 2 && parts[1].Length > 0 ? parts[1] : null };
    }
}
