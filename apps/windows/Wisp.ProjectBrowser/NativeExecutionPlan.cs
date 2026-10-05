using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeExecutionPlan(string Owner, string Key, NativePlanStep[] Steps)
{
    public int Done => Steps.Count(step => step.Status == "done");
    public bool Complete => Steps.Length > 0 && Done == Steps.Length;
    public NativePlanStep? Current => Steps.FirstOrDefault(step => step.Status == "running") ?? Steps.FirstOrDefault(step => step.Status == "pending");
    public string Label(bool busy) => Complete ? "计划已完成" : Current == null ? "计划已结束" : busy ? "执行计划" : "计划未完成";
    public static string StatusLabel(string status) => status switch { "done" => "已完成", "running" => "进行中", "cancelled" => "已取消", _ => "待执行" };
    public static bool Valid(NativePlanStep[] steps) => steps.All(step => step != null && step.Content != null && step.Status is "done" or "running" or "pending" or "cancelled");
    public static NativeExecutionPlan? Latest(ConversationSnapshot? snapshot)
    {
        if (snapshot == null) return null;
        var user = Array.FindLastIndex(snapshot.Items, item => item.Role == "user");
        var turn = (snapshot.UserOffset ?? 0) + snapshot.Items.Count(item => item.Role == "user") - 1;
        var identity = snapshot.HistoryState?.Turns.FirstOrDefault(row => row.UserIndex == turn);
        var owner = $"{snapshot.ProjectId}/{snapshot.SessionId}/{turn}/{identity?.UserSeq}/{identity?.Digest}";
        for (var index = snapshot.Items.Length - 1; index > user; index--)
        {
            var item = snapshot.Items[index];
            if (item.Role != "tool" || item.ToolName != "update_plan" || item.Ok != true) continue;
            // A later successful but unparseable update replaces the old plan.
            if (item.PlanSteps is not { Length: > 0 } steps || !Valid(steps)) return null;
            return new(owner, owner + "/" + JsonSerializer.Serialize(steps), steps);
        }
        return null;
    }
}

public sealed partial class WorkspaceConversationModel
{
    private readonly HashSet<string> dismissedPlans = [];
    public NativeExecutionPlan? ExecutionPlan => ShowingHistory ? null : NativeExecutionPlan.Latest(Snapshot) is { } plan && !dismissedPlans.Contains(plan.Key) ? plan : null;
    public void DismissExecutionPlan(string key)
    {
        if (ExecutionPlan is not { Complete: true } plan || plan.Key != key) return;
        dismissedPlans.Add(key); Notify();
    }
}
