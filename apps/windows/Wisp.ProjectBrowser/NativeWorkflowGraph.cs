using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed record NativeWorkflowNode(string Id, JsonObject Task, int Stage, double X, double Y, int Index);
public sealed record NativeWorkflowEdge(string Source, string Target);
public sealed record NativeWorkflowStage(int Level, double X, int Count);
public sealed record NativeWorkflowGraph(double Width, double Height, NativeWorkflowNode[] Nodes, NativeWorkflowEdge[] Edges, NativeWorkflowStage[] Stages)
{
    public const double NodeWidth = 288, NodeHeight = 196;
    public bool ShowMinimap(int zoom) => Nodes.Length > 8 || zoom < 70;
    public static int CapabilityCount(JsonObject task) => (task["capabilities"] as JsonArray)?.Count ?? 0;
    public static string Role(JsonObject task) => NativeWorkflowStudioModel.Text(task, "task_kind") == "run_activity" ? "计算活动"
        : NativeWorkflowStudioModel.Text(task, "specialist_id") is { Length: > 0 } id ? id : "临时智能体";
    public static string[] Dependencies(JsonObject task) => (task["depends_on"] as JsonArray)?.Select(n => n?.GetValue<string>() ?? "").Where(s => s.Length > 0).Distinct(StringComparer.Ordinal).ToArray() ?? [];
    public static string? Validate(IEnumerable<JsonObject> source, NativeLocalization? localization = null)
    {
        var tasks = source.ToArray(); var ids = tasks.Select(t => NativeWorkflowStudioModel.Text(t, "id")).ToArray();
        if (ids.Any(string.IsNullOrWhiteSpace) || ids.Distinct(StringComparer.Ordinal).Count() != ids.Length) return localization?.Choose("任务 ID 必须非空且唯一。", "Task IDs must be nonempty and unique.") ?? "任务 ID 必须非空且唯一。";
        if (tasks.Any(t => Dependencies(t).Any(d => !ids.Contains(d, StringComparer.Ordinal)))) return localization?.Choose("依赖引用了不存在的任务。", "A dependency refers to a missing task.") ?? "依赖引用了不存在的任务。";
        var remaining = tasks.ToList(); var ready = new HashSet<string>(StringComparer.Ordinal);
        while (remaining.Count > 0)
        {
            var batch = remaining.Where(t => Dependencies(t).All(ready.Contains)).ToArray();
            if (batch.Length == 0) return localization?.Text("workflow_studio.graph_cycle") ?? "任务依赖形成了循环，请移除循环连接。";
            foreach (var task in batch) { ready.Add(NativeWorkflowStudioModel.Text(task, "id")); remaining.Remove(task); }
        }
        return null;
    }
    public static NativeWorkflowGraph Build(IEnumerable<JsonObject> source)
    {
        var tasks = source.ToArray();
        if (tasks.Length == 0) return new(520, 420, [], [], []);
        var ids = tasks.Select(t => NativeWorkflowStudioModel.Text(t, "id")).ToArray();
        var byId = ids.Select((id, index) => (id, index)).GroupBy(v => v.id).ToDictionary(g => g.Key, g => g.First().index, StringComparer.Ordinal);
        var levels = new int[tasks.Length]; var processed = new HashSet<int>();
        while (processed.Count < tasks.Length)
        {
            var ready = Enumerable.Range(0, tasks.Length).Where(i => !processed.Contains(i) && Dependencies(tasks[i]).All(d => !byId.TryGetValue(d, out var index) || processed.Contains(index))).ToArray();
            if (ready.Length == 0)
            { var fallback = levels.Max() + 1; foreach (var i in Enumerable.Range(0, tasks.Length).Where(i => !processed.Contains(i))) levels[i] = fallback; break; }
            foreach (var i in ready)
            { levels[i] = Dependencies(tasks[i]).Where(byId.ContainsKey).Select(d => levels[byId[d]] + 1).DefaultIfEmpty(0).Max(); processed.Add(i); }
        }
        var columns = Enumerable.Range(0, levels.Max() + 1).Select(level => Enumerable.Range(0, tasks.Length).Where(i => levels[i] == level).ToArray()).ToArray();
        var contentHeight = columns.Max(c => c.Length) * NodeHeight + Math.Max(0, columns.Max(c => c.Length) - 1) * 40;
        var height = Math.Max(420, 68 + contentHeight + 28); var offset = (height - (68 + contentHeight + 28)) / 2;
        var nodes = new NativeWorkflowNode[tasks.Length]; var stages = new List<NativeWorkflowStage>();
        foreach (var (column, level) in columns.Select((c, i) => (c, i)))
        {
            var x = 28 + level * (NodeWidth + 112); stages.Add(new(level, x, column.Length));
            var columnHeight = column.Length * NodeHeight + Math.Max(0, column.Length - 1) * 40;
            for (var row = 0; row < column.Length; row++)
            { var i = column[row]; nodes[i] = new(ids[i], (JsonObject)tasks[i].DeepClone(), level, x, 68 + offset + (contentHeight - columnHeight) / 2 + row * (NodeHeight + 40), i); }
        }
        var edges = tasks.SelectMany((t, i) => Dependencies(t).Where(byId.ContainsKey).Select(d => new NativeWorkflowEdge(d, ids[i]))).ToArray();
        return new(Math.Max(520, 56 + columns.Length * NodeWidth + (columns.Length - 1) * 112), height, nodes, edges, stages.ToArray());
    }
    public int FitZoom(double width, double height) => (int)Math.Clamp(Math.Floor(Math.Min(Math.Max(0, width) * .9 / Width, Math.Max(0, height) * .9 / Height) * 100), 25, 140);
}
